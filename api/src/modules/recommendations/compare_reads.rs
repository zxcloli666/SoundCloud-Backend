use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::time::{Duration, Instant};

use futures::StreamExt;
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use uuid::Uuid;

use super::home_artists::{HomeArtist, Shelf, load_artists};
use super::service::util::user_id_variants;
use super::smart_wave::artist_tracks::ArtistTopTracks;
use super::smart_wave::graph::{
    Affinity, GraphEdges, GraphSeed, affinity_from, edge_rows, legacy_edge_rows,
    load_disliked_artists,
};

const TOP_AFFINITY: usize = 300;
const TOP_LIMIT: i64 = 16;
const ADJACENT_LIMIT: i64 = 20;
const FRONTIER: usize = 320;
const SAMPLE_START_PERCENT: f32 = 0.02;
const SAMPLE_MAX_PERCENT: f32 = 100.0;

struct Args {
    database_url: String,
    users: usize,
    k_coplay: Vec<i64>,
    k_colike: Vec<Option<i64>>,
    concurrency: usize,
    timeout_secs: u64,
    worst: usize,
}

impl Args {
    fn parse(raw: Vec<String>) -> Result<Self, String> {
        let mut args = Args {
            database_url: std::env::var("DATABASE_URL").unwrap_or_default(),
            users: 500,
            k_coplay: vec![256],
            k_colike: vec![None],
            concurrency: 1,
            timeout_secs: 30,
            worst: 5,
        };
        let mut raw = raw.into_iter();
        while let Some(flag) = raw.next() {
            let value = raw.next().ok_or(format!("{flag} needs a value"))?;
            match flag.as_str() {
                "--database-url" => args.database_url = value,
                "--users" => args.users = value.parse().map_err(|_| "bad --users")?,
                "--k-coplay" => {
                    args.k_coplay = value
                        .split(',')
                        .map(|k| k.trim().parse().map_err(|_| "bad --k-coplay"))
                        .collect::<Result<_, _>>()?
                }
                "--k-colike" => {
                    args.k_colike = value
                        .split(',')
                        .map(|k| match k.trim() {
                            "none" => Ok(None),
                            k => k.parse().map(Some).map_err(|_| "bad --k-colike"),
                        })
                        .collect::<Result<_, _>>()?
                }
                "--concurrency" => {
                    args.concurrency = value
                        .parse::<usize>()
                        .map_err(|_| "bad --concurrency")?
                        .clamp(1, 2)
                }
                "--timeout-secs" => {
                    args.timeout_secs = value.parse().map_err(|_| "bad --timeout-secs")?
                }
                "--worst" => args.worst = value.parse().map_err(|_| "bad --worst")?,
                other => return Err(format!("unknown flag {other}")),
            }
        }
        if args.database_url.is_empty() {
            return Err("DATABASE_URL or --database-url is required".to_owned());
        }
        Ok(args)
    }
}

#[derive(Clone, Copy, Default)]
struct Agreement {
    jaccard: f64,
    rank: f64,
    identical: bool,
}

fn agreement<T: Eq + std::hash::Hash + Copy>(old: &[T], new: &[T]) -> Agreement {
    let old_set: HashSet<T> = old.iter().copied().collect();
    let new_set: HashSet<T> = new.iter().copied().collect();
    let common = old_set.intersection(&new_set).count();
    let union = old_set.union(&new_set).count();
    let jaccard = if union == 0 {
        1.0
    } else {
        common as f64 / union as f64
    };
    let old_common: Vec<T> = old
        .iter()
        .copied()
        .filter(|x| new_set.contains(x))
        .collect();
    let new_rank: HashMap<T, usize> = new
        .iter()
        .copied()
        .filter(|x| old_set.contains(x))
        .enumerate()
        .map(|(rank, x)| (x, rank))
        .collect();
    let n = old_common.len();
    let rank = if n < 2 {
        1.0
    } else {
        let squares: f64 = old_common
            .iter()
            .enumerate()
            .map(|(rank, x)| {
                let diff = rank as f64 - new_rank[x] as f64;
                diff * diff
            })
            .sum();
        1.0 - 6.0 * squares / (n as f64 * ((n * n) as f64 - 1.0))
    };
    Agreement {
        jaccard,
        rank,
        identical: old == new,
    }
}

fn ranked(affinity: &Affinity) -> Vec<(Uuid, f32)> {
    let mut pairs: Vec<(Uuid, f32)> = affinity.iter().map(|(k, v)| (*k, *v)).collect();
    pairs.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    pairs.truncate(TOP_AFFINITY);
    pairs
}

fn max_value_gap(old: &[(Uuid, f32)], new: &[(Uuid, f32)]) -> f64 {
    let new: HashMap<Uuid, f32> = new.iter().copied().collect();
    old.iter()
        .filter_map(|(artist, value)| new.get(artist).map(|v| f64::from((value - v).abs())))
        .fold(0.0, f64::max)
}

#[derive(Default)]
struct Sample {
    user: String,
    agreement: Agreement,
    value_gap: f64,
}

#[derive(Default)]
struct List {
    samples: Vec<Sample>,
    old_ms: f64,
    new_ms: f64,
    track_pairs: usize,
    track_ties: usize,
    track_diffs: usize,
}

#[derive(Clone, Copy)]
struct Variant {
    coplay: i64,
    colike: Option<i64>,
}

impl Variant {
    fn label(self) -> String {
        match self.colike {
            Some(k) => format!("coplay={} colike={k}", self.coplay),
            None => format!("coplay={} colike=none", self.coplay),
        }
    }
}

struct Probe {
    ms: f64,
    rows: usize,
}

struct ShelfReport {
    name: &'static str,
    artists: Sample,
    noise: Sample,
    old_ms: f64,
    new_ms: f64,
    tracks: [usize; 3],
}

struct UserReport {
    affinity: Vec<(usize, Sample, f64)>,
    probes: Vec<Probe>,
    shelves: Vec<ShelfReport>,
}

async fn legacy_shelf(
    pg: &PgPool,
    shelf: Shelf,
    variants: &[String],
    limit: i64,
    exclude: &[String],
) -> Result<Vec<(Uuid, String)>, sqlx::Error> {
    match shelf {
        Shelf::Top => Ok(sqlx::query_file!(
            "queries/recommendations/home_wave/top_artists_cluster.sql",
            variants,
            limit,
            exclude
        )
        .fetch_all(pg)
        .await?
        .into_iter()
        .map(|row| (row.artist_id, row.sc_track_id))
        .collect()),
        Shelf::Adjacent => Ok(sqlx::query_file!(
            "queries/recommendations/home_wave/adjacent_artists_cluster.sql",
            variants,
            limit,
            exclude
        )
        .fetch_all(pg)
        .await?
        .into_iter()
        .map(|row| (row.artist_id, row.sc_track_id))
        .collect()),
    }
}

async fn plays(pg: &PgPool, ids: &[String]) -> HashMap<String, i64> {
    sqlx::query_as::<_, (String, i64)>(
        "SELECT sc_track_id, COALESCE(play_count, 0) FROM sc_track_counters WHERE sc_track_id = ANY($1)",
    )
    .bind(ids)
    .fetch_all(pg)
    .await
    .unwrap_or_default()
    .into_iter()
    .collect()
}

async fn new_shelf(
    pg: &PgPool,
    tops: &ArtistTopTracks,
    shelf: Shelf,
    variants: &[String],
    limit: i64,
    exclude: &HashSet<String>,
) -> Result<Vec<(Uuid, String)>, sqlx::Error> {
    let artists: Vec<HomeArtist> = load_artists(pg, shelf, variants, limit).await?;
    let ids: Vec<Uuid> = artists.iter().map(|artist| artist.artist_id).collect();
    let best = tops.best_track(pg, &ids, exclude).await;
    Ok(artists
        .into_iter()
        .filter_map(|artist| {
            best.get(&artist.artist_id)
                .map(|track| (artist.artist_id, track.to_string()))
        })
        .collect())
}

async fn compare_user(
    pg: &PgPool,
    old_edges: &GraphEdges,
    new_edges: &[(Variant, GraphEdges)],
    tops: &ArtistTopTracks,
    user: &str,
) -> Result<Option<UserReport>, sqlx::Error> {
    let variants = user_id_variants(user);
    let disliked = load_disliked_artists(pg, &variants).await;
    let old = affinity_from(old_edges, pg, &variants, &disliked, &GraphSeed::User).await;
    if old.is_empty() {
        return Ok(None);
    }
    let old_top = ranked(&old);
    let old_ids: Vec<Uuid> = old_top.iter().map(|(artist, _)| *artist).collect();
    let mut affinity = Vec::new();
    for (index, (_, edges)) in new_edges.iter().enumerate() {
        let started = Instant::now();
        let new = affinity_from(edges, pg, &variants, &disliked, &GraphSeed::User).await;
        let new_top = ranked(&new);
        let new_ids: Vec<Uuid> = new_top.iter().map(|(artist, _)| *artist).collect();
        affinity.push((
            index,
            Sample {
                user: user.to_owned(),
                agreement: agreement(&old_ids, &new_ids),
                value_gap: max_value_gap(&old_top, &new_top),
            },
            started.elapsed().as_secs_f64() * 1000.0,
        ));
    }

    let frontier: Vec<Uuid> = old_ids.iter().copied().take(FRONTIER).collect();
    let mut probes = Vec::new();
    for _ in 0..2 {
        probes.clear();
        let started = Instant::now();
        let rows = legacy_edge_rows(pg, &frontier).await.unwrap_or(0);
        probes.push(Probe {
            ms: started.elapsed().as_secs_f64() * 1000.0,
            rows,
        });
        for (variant, _) in new_edges {
            let started = Instant::now();
            let rows = edge_rows(pg, &frontier, variant.coplay, variant.colike)
                .await
                .unwrap_or(0);
            probes.push(Probe {
                ms: started.elapsed().as_secs_f64() * 1000.0,
                rows,
            });
        }
    }

    let mut shelves = Vec::new();
    for (name, shelf, limit) in [
        ("home top_artists", Shelf::Top, TOP_LIMIT),
        ("home adjacent", Shelf::Adjacent, ADJACENT_LIMIT),
    ] {
        let started = Instant::now();
        let old_rows = legacy_shelf(pg, shelf, &variants, limit, &[]).await?;
        let old_ms = started.elapsed().as_secs_f64() * 1000.0;
        let again = legacy_shelf(pg, shelf, &variants, limit, &[]).await?;
        let started = Instant::now();
        let new_rows = new_shelf(pg, tops, shelf, &variants, limit, &HashSet::new()).await?;
        let new_ms = started.elapsed().as_secs_f64() * 1000.0;

        let artists = |rows: &[(Uuid, String)]| rows.iter().map(|(a, _)| *a).collect::<Vec<_>>();
        let baseline = Sample {
            user: user.to_owned(),
            agreement: agreement(&artists(&old_rows), &artists(&again)),
            value_gap: 0.0,
        };
        let artist_sample = Sample {
            user: user.to_owned(),
            agreement: agreement(&artists(&old_rows), &artists(&new_rows)),
            value_gap: 0.0,
        };

        let new_track: HashMap<Uuid, &String> = new_rows.iter().map(|(a, t)| (*a, t)).collect();
        let mut differing = Vec::new();
        let mut pairs = 0;
        for (artist, track) in &old_rows {
            if let Some(other) = new_track.get(artist) {
                pairs += 1;
                if track != *other {
                    differing.push((track.clone(), (*other).clone()));
                }
            }
        }
        let wanted: Vec<String> = differing
            .iter()
            .flat_map(|(a, b)| [a.clone(), b.clone()])
            .collect();
        let counts = plays(pg, &wanted).await;
        let ties = differing
            .iter()
            .filter(|(a, b)| {
                counts.get(a).copied().unwrap_or(0) == counts.get(b).copied().unwrap_or(0)
            })
            .count();
        shelves.push(ShelfReport {
            name,
            artists: artist_sample,
            noise: baseline,
            old_ms,
            new_ms,
            tracks: [pairs, differing.len(), ties],
        });
    }
    Ok(Some(UserReport {
        affinity,
        probes,
        shelves,
    }))
}

async fn sample_users(pg: &PgPool, wanted: usize) -> Result<Vec<String>, sqlx::Error> {
    let mut percent = SAMPLE_START_PERCENT;
    loop {
        let users: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT user_id FROM user_likes_tracks TABLESAMPLE SYSTEM ($1)
             WHERE wanted_state = true AND created_at > now() - interval '180 days'
             LIMIT $2",
        )
        .bind(percent)
        .bind(i64::try_from(wanted * 2).unwrap_or(i64::MAX))
        .fetch_all(pg)
        .await?;
        if users.len() >= wanted * 2 || percent >= SAMPLE_MAX_PERCENT {
            return Ok(users);
        }
        percent = (percent * 4.0).min(SAMPLE_MAX_PERCENT);
    }
}

fn percentiles(values: &mut [f64]) -> String {
    if values.is_empty() {
        return "-".to_owned();
    }
    values.sort_by(f64::total_cmp);
    let at = |q: f64| values[((values.len() - 1) as f64 * q).round() as usize];
    format!(
        "{:.0} {:.0} {:.0}",
        at(0.5),
        at(0.95),
        values[values.len() - 1]
    )
}

fn mean(values: impl Iterator<Item = f64>) -> f64 {
    let (sum, count) = values.fold((0.0, 0usize), |(s, c), v| (s + v, c + 1));
    if count == 0 { 0.0 } else { sum / count as f64 }
}

fn print_list(title: &str, list: &List, worst: usize, with_gap: bool) {
    let n = list.samples.len();
    if n == 0 {
        println!("{title}: no samples");
        return;
    }
    let identical = list
        .samples
        .iter()
        .filter(|s| s.agreement.identical)
        .count();
    let min_jaccard = list
        .samples
        .iter()
        .map(|s| s.agreement.jaccard)
        .fold(1.0, f64::min);
    let min_rank = list
        .samples
        .iter()
        .map(|s| s.agreement.rank)
        .fold(1.0, f64::min);
    println!(
        "{title}: users={n} identical={identical} jaccard mean={:.4} min={:.4} rank mean={:.4} min={:.4} ms old={:.0} new={:.0}",
        mean(list.samples.iter().map(|s| s.agreement.jaccard)),
        min_jaccard,
        mean(list.samples.iter().map(|s| s.agreement.rank)),
        min_rank,
        list.old_ms / n as f64,
        list.new_ms / n as f64,
    );
    if with_gap {
        println!(
            "  value gap on common artists: mean max={:.5} worst={:.5}",
            mean(list.samples.iter().map(|s| s.value_gap)),
            list.samples.iter().map(|s| s.value_gap).fold(0.0, f64::max)
        );
    }
    if list.track_pairs > 0 {
        println!(
            "  tracks: common artists={} different track={} of them equal play_count={} real={}",
            list.track_pairs,
            list.track_diffs,
            list.track_ties,
            list.track_diffs - list.track_ties
        );
    }
    let mut order: Vec<&Sample> = list.samples.iter().collect();
    order.sort_by(|a, b| {
        a.agreement
            .jaccard
            .total_cmp(&b.agreement.jaccard)
            .then(a.agreement.rank.total_cmp(&b.agreement.rank))
    });
    for sample in order.into_iter().take(worst) {
        println!(
            "  worst user={} jaccard={:.4} rank={:.4} value_gap={:.5}",
            sample.user, sample.agreement.jaccard, sample.agreement.rank, sample.value_gap
        );
    }
}

pub async fn run(raw: Vec<String>) -> i32 {
    let args = match Args::parse(raw) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("compare-reads: {message}");
            eprintln!(
                "usage: api compare-reads [--database-url URL] [--users 500] [--k-coplay 256[,...]] [--k-colike 1024,2048,4096,none] [--concurrency 1|2] [--timeout-secs 30] [--worst 5]"
            );
            return 2;
        }
    };
    match execute(args).await {
        Ok(()) => 0,
        Err(message) => {
            eprintln!("compare-reads: {message}");
            1
        }
    }
}

async fn execute(args: Args) -> Result<(), String> {
    let options = PgConnectOptions::from_str(&args.database_url).map_err(|e| e.to_string())?;
    let timeout_ms = args.timeout_secs * 1000;
    let pg = PgPoolOptions::new()
        .max_connections(u32::try_from(args.concurrency + 1).unwrap_or(2))
        .acquire_timeout(Duration::from_secs(30))
        .after_connect(move |conn, _| {
            Box::pin(async move {
                sqlx::query("SET default_transaction_read_only = on")
                    .execute(&mut *conn)
                    .await?;
                sqlx::query(&format!("SET statement_timeout = {timeout_ms}"))
                    .execute(&mut *conn)
                    .await?;
                Ok(())
            })
        })
        .connect_with(options)
        .await
        .map_err(|e| e.to_string())?;
    let read_only: String = sqlx::query_scalar("SHOW default_transaction_read_only")
        .fetch_one(&pg)
        .await
        .map_err(|e| e.to_string())?;
    if read_only != "on" {
        return Err("the session is not read only, refusing to run".to_owned());
    }

    let candidates = sample_users(&pg, args.users)
        .await
        .map_err(|e| e.to_string())?;
    println!(
        "candidates={} wanted={} k_coplay={:?} k_colike={:?} concurrency={}",
        candidates.len(),
        args.users,
        args.k_coplay,
        args.k_colike,
        args.concurrency
    );

    let old_edges = GraphEdges::with_top_k(i64::MAX, None);
    let new_edges: Vec<(Variant, GraphEdges)> = args
        .k_coplay
        .iter()
        .flat_map(|coplay| {
            args.k_colike.iter().map(move |colike| Variant {
                coplay: *coplay,
                colike: *colike,
            })
        })
        .map(|variant| {
            (
                variant,
                GraphEdges::with_top_k(variant.coplay, variant.colike),
            )
        })
        .collect();
    let tops = ArtistTopTracks::for_home();

    let mut stream = futures::stream::iter(candidates)
        .map(|user| {
            let (pg, old_edges, new_edges, tops) = (&pg, &old_edges, &new_edges, &tops);
            async move {
                let report = compare_user(pg, old_edges, new_edges, tops, &user).await;
                (user, report)
            }
        })
        .buffer_unordered(args.concurrency);

    let mut affinity: HashMap<usize, List> = HashMap::new();
    let mut edge_probes: Vec<Vec<(f64, usize)>> = vec![Vec::new(); new_edges.len() + 1];
    let mut shelves: HashMap<&'static str, (List, List)> = HashMap::new();
    let (mut used, mut empty, mut failed) = (0usize, 0usize, 0usize);
    while let Some((user, report)) = stream.next().await {
        if used >= args.users {
            break;
        }
        match report {
            Ok(Some(report)) => {
                used += 1;
                for (slot, probe) in report.probes.iter().enumerate() {
                    edge_probes[slot].push((probe.ms, probe.rows));
                }
                for (index, sample, ms) in report.affinity {
                    let list = affinity.entry(index).or_default();
                    list.new_ms += ms;
                    list.samples.push(sample);
                }
                for shelf in report.shelves {
                    let (list, noise) = shelves.entry(shelf.name).or_default();
                    list.old_ms += shelf.old_ms;
                    list.new_ms += shelf.new_ms;
                    list.track_pairs += shelf.tracks[0];
                    list.track_diffs += shelf.tracks[1];
                    list.track_ties += shelf.tracks[2];
                    list.samples.push(shelf.artists);
                    noise.samples.push(shelf.noise);
                }
            }
            Ok(None) => empty += 1,
            Err(error) => {
                failed += 1;
                eprintln!("user {user}: {error}");
            }
        }
    }
    println!("compared={used} without_signal={empty} failed={failed}");
    println!("affinity top {TOP_AFFINITY}: old = every edge, new = top K per artist and branch");
    for (index, (variant, _)) in new_edges.iter().enumerate() {
        if let Some(list) = affinity.get(&index) {
            print_list(
                &format!("affinity {}", variant.label()),
                list,
                args.worst,
                true,
            );
        }
    }
    println!(
        "edge query on the 320 heaviest artists of each user, warm, per user: ms and rows p50 p95 max"
    );
    let labels = std::iter::once("old".to_owned())
        .chain(new_edges.iter().map(|(variant, _)| variant.label()));
    for (label, probes) in labels.zip(&edge_probes) {
        let mut ms: Vec<f64> = probes.iter().map(|p| p.0).collect();
        let mut rows: Vec<f64> = probes.iter().map(|p| p.1 as f64).collect();
        println!(
            "  {label}: ms {} rows {}",
            percentiles(&mut ms),
            percentiles(&mut rows)
        );
    }
    println!(
        "home shelves: old = legacy SQL, new = artist list + ArtistTopTracks; noise = old SQL against itself"
    );
    let mut names: Vec<&&'static str> = shelves.keys().collect();
    names.sort();
    for name in names {
        let (list, noise) = &shelves[*name];
        print_list(&format!("{name} artists"), list, args.worst, false);
        print_list(&format!("{name} noise"), noise, 0, false);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn fixture(pool: &PgPool) -> anyhow::Result<()> {
        let statements = [
            "INSERT INTO artists (id, name, normalized_name, source)
             SELECT lpad(to_hex(n), 32, '0')::uuid, 'a' || n, 'a' || n, 'test'
             FROM generate_series(1, 60) AS n",
            "UPDATE artists SET merged_into = lpad(to_hex(1), 32, '0')::uuid WHERE name IN ('a25', 'a26')",
            "WITH made AS (
                 INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms,
                                     sharing, storage_state, index_state, needs_duration_resolve)
                 SELECT (a * 100 + t)::text, 'urn:' || (a * 100 + t), 'T', 't', 1000,
                        CASE WHEN a % 5 = 0 AND t = 10 THEN 'private' ELSE 'public' END,
                        CASE WHEN a % 9 = 0 AND t = 10 THEN 'pending' ELSE 'ok' END,
                        CASE WHEN a % 3 = 0 AND t = 8 THEN 'pending' ELSE 'indexed' END,
                        a % 4 = 0 AND t = 9
                 FROM generate_series(1, 60) AS a, generate_series(1, 10) AS t
                 RETURNING id, sc_track_id
             ), credited AS (
                 INSERT INTO track_artists (track_id, artist_id, role, source)
                 SELECT id, lpad(to_hex(sc_track_id::int / 100), 32, '0')::uuid, 'primary', 'test'
                 FROM made
             )
             INSERT INTO sc_track_counters (sc_track_id, play_count)
             SELECT sc_track_id, (sc_track_id::int % 100) * 10 FROM made",
            "INSERT INTO artist_coplay (a_id, b_id, weight)
             SELECT LEAST(lpad(to_hex(u), 32, '0')::uuid, lpad(to_hex(c), 32, '0')::uuid),
                    GREATEST(lpad(to_hex(u), 32, '0')::uuid, lpad(to_hex(c), 32, '0')::uuid),
                    1 + (u * 100 + c) / 10000.0
             FROM generate_series(1, 12) AS u, generate_series(21, 50) AS c
             WHERE (u * c) % 3 <> 0",
            "INSERT INTO artist_coplay (a_id, b_id, weight)
             SELECT lpad(to_hex(u1), 32, '0')::uuid, lpad(to_hex(u2), 32, '0')::uuid,
                    100 + (u1 * 100 + u2) / 10000.0
             FROM generate_series(1, 12) AS u1, generate_series(1, 12) AS u2
             WHERE u1 < u2",
            "INSERT INTO user_likes_tracks (user_id, sc_track_id, wanted_state, created_at)
             SELECT 'u2', (a * 100 + 1)::text, true, now() - interval '1 hour'
             FROM generate_series(1, 12) AS a",
            "INSERT INTO user_likes_tracks (user_id, sc_track_id, wanted_state, created_at)
             SELECT 'u1', (a * 100 + t)::text, true, now() - interval '1 hour'
             FROM (VALUES (13, 5), (14, 6)) AS likes(a, n), generate_series(1, 10) AS t
             WHERE t <= n",
            "INSERT INTO user_events (sc_user_id, sc_track_id, event_type, weight, created_at)
             SELECT 'u1', (a * 100 + t)::text, 'full_play', 1.0, now() - interval '1 day'
             FROM (VALUES (15, 3), (16, 5), (17, 7), (18, 9)) AS plays(a, n),
                  generate_series(1, 10) AS t
             WHERE t <= n",
        ];
        for statement in statements {
            sqlx::query(statement).execute(pool).await?;
        }
        Ok(())
    }

    fn excluded() -> Vec<String> {
        let mut ids: Vec<String> = [31, 32, 34, 35, 37, 38]
            .iter()
            .flat_map(|a| [a * 100 + 10, a * 100 + 9])
            .map(|id| id.to_string())
            .collect();
        ids.extend((1..=10).map(|t| (44 * 100 + t).to_string()));
        ids
    }

    async fn same_rows(pool: &PgPool, shelf: Shelf, user: &str, limit: i64) -> anyhow::Result<()> {
        let variants = vec![user.to_owned()];
        for exclude in [Vec::new(), excluded()] {
            let tops = ArtistTopTracks::for_home();
            let old = legacy_shelf(pool, shelf, &variants, limit, &exclude).await?;
            let set: HashSet<String> = exclude.iter().cloned().collect();
            let new = new_shelf(pool, &tops, shelf, &variants, limit, &set).await?;
            assert!(!old.is_empty(), "{user} {limit}");
            assert_eq!(new, old, "{user} {limit} {}", exclude.len());
        }
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_top_artists_shelf_is_what_the_legacy_query_returned(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        fixture(&pool).await?;
        for limit in [16, 4] {
            same_rows(&pool, Shelf::Top, "u1", limit).await?;
        }
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_adjacent_shelf_is_what_the_legacy_query_returned(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        fixture(&pool).await?;
        for limit in [20, 7] {
            same_rows(&pool, Shelf::Adjacent, "u2", limit).await?;
        }
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_adjacent_edges_use_the_covering_indexes(pool: PgPool) -> anyhow::Result<()> {
        fixture(&pool).await?;
        sqlx::query("ANALYZE artist_coplay").execute(&pool).await?;
        let mut tx = pool.begin().await?;
        sqlx::query("SET LOCAL enable_seqscan = off")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL enable_sort = off")
            .execute(&mut *tx)
            .await?;
        let plan: Vec<String> = sqlx::query_scalar(&format!(
            "EXPLAIN (COSTS OFF) {}",
            include_str!("../../../queries/recommendations/home_wave/adjacent_artists.sql")
        ))
        .bind(vec!["u2".to_owned()])
        .bind(20_i64)
        .fetch_all(&mut *tx)
        .await?;
        let plan = plan.join("\n");
        assert!(plan.contains("artist_coplay_a_edges_idx"), "{plan}");
        assert!(plan.contains("artist_coplay_b_edges_idx"), "{plan}");
        Ok(())
    }

    #[test]
    fn agreement_counts_overlap_and_order() {
        let same = agreement(&[1, 2, 3], &[1, 2, 3]);
        assert!(same.identical && same.jaccard == 1.0 && same.rank == 1.0);
        let swapped = agreement(&[1, 2, 3], &[3, 2, 1]);
        assert!(!swapped.identical && swapped.jaccard == 1.0 && swapped.rank == -1.0);
        let partial = agreement(&[1, 2, 3, 4], &[1, 2, 5, 6]);
        assert!((partial.jaccard - 1.0 / 3.0).abs() < 1e-9);
        assert_eq!(partial.rank, 1.0);
    }
}
