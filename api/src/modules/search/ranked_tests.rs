use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use super::SearchService;
use super::query::TrackSearchQuery;
use super::ranked;
use crate::cache::CacheService;

const TITLE_TOKENS: &str =
    include_str!("../../../queries/search/ranked/candidates_title_tokens.sql");

fn ranked_service(pool: &PgPool) -> anyhow::Result<std::sync::Arc<SearchService>> {
    let redis = deadpool_redis::Config::from_url("redis://127.0.0.1:1")
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    Ok(SearchService::new(
        pool.clone(),
        CacheService::new(redis),
        true,
    ))
}

async fn artist(pool: &PgPool, name: &str, normalized: &str) -> anyhow::Result<Uuid> {
    Ok(sqlx::query_scalar(
        "INSERT INTO artists (name, normalized_name, source, track_count_primary, monthly_listeners)
         VALUES ($1, $2, 'test', 1, 1000) RETURNING id",
    )
    .bind(name)
    .bind(normalized)
    .fetch_one(pool)
    .await?)
}

struct Upload<'a> {
    id: &'a str,
    title: &'a str,
    uploader: &'a str,
    uploader_id: &'a str,
    plays: i64,
    artist: Option<Uuid>,
}

async fn upload(pool: &PgPool, track: Upload<'_>) -> anyhow::Result<Uuid> {
    Ok(sqlx::query_scalar(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, sharing,
                             play_count_sc, uploader_sc_user_id, uploader_username, primary_artist_id)
         VALUES ($1, 'soundcloud:tracks:' || $1, $2, $3, 180000, 'public', $4, $5, $6, $7)
         RETURNING id",
    )
    .bind(track.id)
    .bind(track.title)
    .bind(catalog_normalize::normalize_title(track.title))
    .bind(track.plays)
    .bind(track.uploader_id)
    .bind(track.uploader)
    .bind(track.artist)
    .fetch_one(pool)
    .await?)
}

async fn first_titles(
    search: &SearchService,
    q: &str,
) -> anyhow::Result<(Vec<String>, Option<bool>)> {
    let found = search
        .track_page(
            &TrackSearchQuery {
                q: Some(q.into()),
                ..Default::default()
            },
            0,
            20,
        )
        .await?;
    let titles = found
        .page
        .collection
        .iter()
        .map(|item| {
            format!(
                "{} / {}",
                item["title"].as_str().unwrap_or_default(),
                item["user"]["username"].as_str().unwrap_or_default()
            )
        })
        .collect();
    Ok((titles, found.weak))
}

#[sqlx::test(migrations = "./migrations")]
async fn an_artist_and_title_query_finds_the_official_upload_in_either_order(
    pool: PgPool,
) -> anyhow::Result<()> {
    let juice = artist(&pool, "Juice WRLD", "juice wrld").await?;
    upload(
        &pool,
        Upload {
            id: "1",
            title: "Lucid Dreams",
            uploader: "Juice WRLD",
            uploader_id: "100",
            plays: 400_000_000,
            artist: Some(juice),
        },
    )
    .await?;
    upload(
        &pool,
        Upload {
            id: "2",
            title: "Juice WRLD - Lucid Dreams",
            uploader: "lyricsvault",
            uploader_id: "200",
            plays: 900_000_000,
            artist: Some(juice),
        },
    )
    .await?;
    upload(
        &pool,
        Upload {
            id: "3",
            title: "Robbery",
            uploader: "Juice WRLD",
            uploader_id: "100",
            plays: 300_000_000,
            artist: Some(juice),
        },
    )
    .await?;
    let search = ranked_service(&pool)?;

    for q in ["juice wrld lucid dreams", "lucid dreams juice wrld"] {
        let (titles, weak) = first_titles(&search, q).await?;
        assert_eq!(titles[0], "Lucid Dreams / Juice WRLD", "{q}: {titles:?}");
        assert!(
            titles.contains(&"Juice WRLD - Lucid Dreams / lyricsvault".to_owned()),
            "{q}: {titles:?}"
        );
        assert!(weak.is_some(), "a ranked page says whether it is weak");
    }
    let (titles, _) = first_titles(&search, "juice wrld").await?;
    assert_eq!(
        titles.len(),
        3,
        "an artist alone lists the artist's uploads"
    );

    let found = search
        .track_page(
            &TrackSearchQuery {
                q: Some("juice wrld lucid dreams".into()),
                ..Default::default()
            },
            0,
            20,
        )
        .await?;
    let scores: Vec<f64> = found
        .page
        .collection
        .iter()
        .map(|item| {
            item["_scd_search"]["score"]
                .as_f64()
                .expect("every ranked row carries its score")
        })
        .collect();
    assert!(
        scores.windows(2).all(|pair| pair[0] >= pair[1]),
        "rows come best first: {scores:?}"
    );
    assert!(scores[0] >= 0.8, "{scores:?}");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_featured_credit_and_a_label_upload_are_found_through_the_artist(
    pool: PgPool,
) -> anyhow::Result<()> {
    let savage = artist(&pool, "21 Savage", "21 savage").await?;
    let post = artist(&pool, "Post Malone", "post malone").await?;
    let rockstar = upload(
        &pool,
        Upload {
            id: "10",
            title: "rockstar (feat. 21 Savage)",
            uploader: "Post Malone",
            uploader_id: "300",
            plays: 800_000_000,
            artist: Some(post),
        },
    )
    .await?;
    sqlx::query(
        "INSERT INTO track_artists (track_id, artist_id, role, source) VALUES ($1, $2, 'featured', 'test')",
    )
    .bind(rockstar)
    .bind(savage)
    .execute(&pool)
    .await?;
    let kavinsky = artist(&pool, "Kavinsky", "kavinsky").await?;
    upload(
        &pool,
        Upload {
            id: "11",
            title: "Nightcall",
            uploader: "Record Makers",
            uploader_id: "400",
            plays: 50_000_000,
            artist: Some(kavinsky),
        },
    )
    .await?;
    let search = ranked_service(&pool)?;

    let (titles, _) = first_titles(&search, "rockstar 21 savage").await?;
    assert_eq!(
        titles.first().map(String::as_str),
        Some("rockstar (feat. 21 Savage) / Post Malone")
    );
    let (titles, _) = first_titles(&search, "kavinsky nightcall").await?;
    assert_eq!(
        titles.first().map(String::as_str),
        Some("Nightcall / Record Makers")
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_cyrillic_artist_query_finds_its_upload_and_latin_spelling_too(
    pool: PgPool,
) -> anyhow::Result<()> {
    let band = artist(&pool, "Король и Шут", "король и шут").await?;
    upload(
        &pool,
        Upload {
            id: "20",
            title: "Кукла колдуньи",
            uploader: "Король и Шут",
            uploader_id: "500",
            plays: 30_000_000,
            artist: Some(band),
        },
    )
    .await?;
    upload(
        &pool,
        Upload {
            id: "21",
            title: "Король и Шут - Кукла Колдуньи (cover)",
            uploader: "guitarboy",
            uploader_id: "501",
            plays: 40_000_000,
            artist: None,
        },
    )
    .await?;
    upload(
        &pool,
        Upload {
            id: "22",
            title: "Kishlak - Platina",
            uploader: "kishlak",
            uploader_id: "502",
            plays: 1_000_000,
            artist: None,
        },
    )
    .await?;
    let search = ranked_service(&pool)?;

    let (titles, _) = first_titles(&search, "король и шут кукла колдуньи").await?;
    assert_eq!(titles[0], "Кукла колдуньи / Король и Шут", "{titles:?}");
    let (titles, _) = first_titles(&search, "кишлак платина").await?;
    assert_eq!(
        titles,
        ["Kishlak - Platina / kishlak"],
        "a Cyrillic query retries with its Latin spelling"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn the_artist_strip_puts_an_exact_span_first(pool: PgPool) -> anyhow::Result<()> {
    artist(&pool, "Juice WRLD", "juice wrld").await?;
    let search = ranked_service(&pool)?;
    let found = search.artists("juice wrld lucid dreams", 0, 20).await?;
    assert_eq!(found.collection.len(), 1);
    assert_eq!(found.collection[0]["name"], "Juice WRLD");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_ranked_slot_is_served_by_its_winner_and_a_deleted_one_is_dropped(
    pool: PgPool,
) -> anyhow::Result<()> {
    let mut ids = Vec::new();
    for n in 1..=3 {
        ids.push(
            upload(
                &pool,
                Upload {
                    id: &n.to_string(),
                    title: &format!("Song {n}"),
                    uploader: "someone",
                    uploader_id: "9",
                    plays: n,
                    artist: None,
                },
            )
            .await?,
        );
    }
    sqlx::query("UPDATE tracks SET superseded_by = $1 WHERE id = $2")
        .bind(ids[2])
        .bind(ids[0])
        .execute(&pool)
        .await?;
    sqlx::query("UPDATE tracks SET deleted_at = now() WHERE id = $1")
        .bind(ids[1])
        .execute(&pool)
        .await?;

    let served = ranked::serving_rows(&pool, &ids).await?;
    let urns: Vec<&str> = served.iter().map(|row| row.urn.as_str()).collect();
    assert_eq!(urns, ["soundcloud:tracks:3"]);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_multi_word_query_that_matches_nothing_stays_on_the_trigram_index(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, sharing, play_count_sc)
         SELECT n::text, 'soundcloud:tracks:' || n,
                (ARRAY['ocean drift', 'silent road', 'ocean road'])[1 + n % 3] || ' ' || n,
                (ARRAY['ocean drift', 'silent road', 'ocean road'])[1 + n % 3] || ' ' || n,
                120000, 'public', n
         FROM generate_series(1, 60000) AS n;
         ANALYZE tracks;",
    )
    .execute(&pool)
    .await?;

    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('plan_cache_mode', 'force_custom_plan', true)")
        .execute(&mut *tx)
        .await?;
    let plan: Value = sqlx::query_scalar(&format!("EXPLAIN (FORMAT JSON) {TITLE_TOKENS}"))
        .bind("%dreams%")
        .bind(Some("%juice%"))
        .bind(Some("%lucid%"))
        .bind(Some("%wrld%"))
        .fetch_one(&mut *tx)
        .await?;
    let plan = plan.to_string();
    assert!(
        plan.contains("tracks_search_title_norm_trgm"),
        "title tokens must be found by trigram: {plan}"
    );
    assert!(
        !plan.contains("tracks_public_popular_idx"),
        "a query without matches must never walk the popularity index: {plan}"
    );
    Ok(())
}
