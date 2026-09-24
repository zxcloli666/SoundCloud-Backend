use std::collections::HashMap;

use rand::distributions::Distribution;
use rand_distr::Beta;
use sqlx::PgPool;

use crate::error::AppResult;

const PRIOR_ALPHA: f64 = 1.0;
const PRIOR_BETA: f64 = 1.0;
const CLICK_WEIGHT: f64 = 0.4;
const COMPLETE_WEIGHT: f64 = 0.6;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ClusterStat {
    pub cluster_id: String,
    pub shows: i64,
    pub clicks: i64,
    pub completes: i64,
}

pub async fn load_stats(pg: &PgPool, sc_user_id: &str) -> AppResult<HashMap<String, ClusterStat>> {
    let variants = crate::common::sc_ids::user_id_variants(sc_user_id);
    let rows: Vec<ClusterStat> =
        sqlx::query_file!("queries/recommendations/bandits/load_stats.sql", &variants)
            .fetch_all(pg)
            .await?
            .into_iter()
            .map(|r| ClusterStat {
                cluster_id: r.cluster_id,
                shows: r.shows,
                clicks: r.clicks,
                completes: r.completes,
            })
            .collect();
    Ok(rows
        .into_iter()
        .map(|r| (r.cluster_id.clone(), r))
        .collect())
}

pub fn sample_priorities(stats: &HashMap<String, ClusterStat>, clusters: &[&str]) -> Vec<f64> {
    let mut rng = rand::thread_rng();
    clusters
        .iter()
        .map(|c| {
            let stat = stats.get(*c);
            let (alpha, beta) = match stat {
                Some(s) => {
                    let positive =
                        (s.clicks as f64) * CLICK_WEIGHT + (s.completes as f64) * COMPLETE_WEIGHT;
                    let negative = (s.shows as f64 - positive).max(0.0);
                    (PRIOR_ALPHA + positive, PRIOR_BETA + negative)
                }
                None => (PRIOR_ALPHA, PRIOR_BETA),
            };
            match Beta::new(alpha, beta) {
                Ok(d) => d.sample(&mut rng),
                Err(_) => 0.5,
            }
        })
        .collect()
}

pub fn order_by_thompson<'a>(
    clusters: &'a [&'a str],
    stats: &HashMap<String, ClusterStat>,
) -> Vec<&'a str> {
    let priorities = sample_priorities(stats, clusters);
    let mut indexed: Vec<(usize, f64)> = priorities.into_iter().enumerate().collect();
    indexed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    indexed.into_iter().map(|(i, _)| clusters[i]).collect()
}

pub async fn record_shows(
    pg: &PgPool,
    sc_user_id: &str,
    counts: &[(String, i64)],
) -> AppResult<()> {
    let mut clusters: Vec<&str> = Vec::new();
    let mut shows: Vec<i64> = Vec::new();
    for (cluster, n) in counts {
        if *n > 0 {
            clusters.push(cluster.as_str());
            shows.push(*n);
        }
    }
    if clusters.is_empty() {
        return Ok(());
    }
    sqlx::query_file!(
        "queries/recommendations/bandits/record_shows.sql",
        sc_user_id,
        &clusters as &[&str],
        &shows
    )
    .execute(pg)
    .await?;
    Ok(())
}

pub async fn record_outcome(
    pg: &PgPool,
    sc_user_id: &str,
    cluster_id: &str,
    clicks: i64,
    completes: i64,
) -> AppResult<()> {
    sqlx::query_file!(
        "queries/recommendations/bandits/record_outcome.sql",
        sc_user_id,
        cluster_id,
        clicks,
        completes
    )
    .execute(pg)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRAWS: usize = 600;

    fn stat(cluster: &str, shows: i64, clicks: i64, completes: i64) -> (String, ClusterStat) {
        (
            cluster.to_owned(),
            ClusterStat {
                cluster_id: cluster.to_owned(),
                shows,
                clicks,
                completes,
            },
        )
    }

    fn mean_priority(stats: &HashMap<String, ClusterStat>, cluster: &str) -> f64 {
        let clusters = [cluster];
        let total: f64 = (0..DRAWS)
            .map(|_| sample_priorities(stats, &clusters)[0])
            .sum();
        total / DRAWS as f64
    }

    #[test]
    fn a_cluster_nobody_has_seen_starts_at_the_uniform_prior() {
        let mean = mean_priority(&HashMap::new(), "unknown");

        assert!(
            (mean - 0.5).abs() < 0.05,
            "an unseen cluster must start indifferent, saw {mean}"
        );
    }

    #[test]
    fn a_cluster_people_finish_outranks_one_they_skip() {
        let stats: HashMap<String, ClusterStat> =
            [stat("loved", 100, 0, 90), stat("skipped", 100, 0, 0)]
                .into_iter()
                .collect();
        let clusters = ["skipped", "loved"];

        let wins = (0..200)
            .filter(|_| order_by_thompson(&clusters, &stats)[0] == "loved")
            .count();

        assert!(
            wins >= 190,
            "a cluster people finish must lead nearly always, led {wins} of 200"
        );
    }

    #[test]
    fn a_finish_is_worth_more_than_a_click() {
        let stats: HashMap<String, ClusterStat> =
            [stat("clicked", 100, 100, 0), stat("finished", 100, 0, 100)]
                .into_iter()
                .collect();

        let clicked = mean_priority(&stats, "clicked");
        let finished = mean_priority(&stats, "finished");

        assert!(
            finished > clicked + 0.1,
            "finishing must weigh more than clicking, saw {finished} against {clicked}"
        );
    }

    #[test]
    fn a_counter_that_overruns_its_shows_cannot_break_the_draw() {
        let stats: HashMap<String, ClusterStat> =
            [stat("broken", 1, 10_000, 10_000)].into_iter().collect();

        let mean = mean_priority(&stats, "broken");

        assert!(
            mean.is_finite() && (0.0..=1.0).contains(&mean),
            "a broken counter must stay inside the unit interval, saw {mean}"
        );
        assert!(
            mean > 0.9,
            "clamping the negative side keeps the draw usable instead of falling back to a \
             constant, saw {mean}"
        );
    }

    #[test]
    fn the_order_keeps_every_cluster_exactly_once() {
        let stats: HashMap<String, ClusterStat> = [stat("a", 10, 5, 5)].into_iter().collect();
        let clusters = ["a", "b", "c", "d"];

        for _ in 0..50 {
            let mut ordered = order_by_thompson(&clusters, &stats);
            assert_eq!(ordered.len(), clusters.len());
            ordered.sort_unstable();
            assert_eq!(
                ordered, clusters,
                "a cluster must not be lost or duplicated"
            );
        }
    }

    #[tokio::test]
    async fn nothing_to_record_never_reaches_the_database() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_millis(50))
            .connect_lazy("postgres://nobody:nobody@127.0.0.1:1/nothing")
            .expect("a lazy pool never dials");

        record_shows(&pool, "17", &[])
            .await
            .expect("empty is a no-op");
        record_shows(&pool, "17", &[("cluster".to_owned(), 0)])
            .await
            .expect("a zero count is a no-op");
    }
}
