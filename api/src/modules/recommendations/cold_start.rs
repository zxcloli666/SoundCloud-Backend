use sqlx::PgPool;

use crate::error::AppResult;

use super::service::RecommendationsService;

const FRESH_DAYS: i32 = 14;
const POOL_FRESH: i64 = 80;
const POOL_POPULAR: i64 = 80;

impl RecommendationsService {
    pub async fn cold_start_pool(
        &self,
        languages: Option<&[String]>,
        limit: usize,
    ) -> AppResult<Vec<String>> {
        let lang_filter: Option<Vec<String>> = languages.map(|v| v.to_vec());

        let (fresh, popular) = tokio::join!(
            load_fresh(&self.pg, lang_filter.as_deref()),
            load_popular(&self.pg, lang_filter.as_deref())
        );
        let fresh = fresh.unwrap_or_default();
        let popular = popular.unwrap_or_default();

        let mut combined: Vec<String> = Vec::with_capacity(fresh.len() + popular.len());
        let mut seen = std::collections::HashSet::new();
        let mut fi = 0;
        let mut pi = 0;
        while combined.len() < limit * 4 && (fi < fresh.len() || pi < popular.len()) {
            if fi < fresh.len() {
                let id = &fresh[fi];
                if seen.insert(id.clone()) {
                    combined.push(id.clone());
                }
                fi += 1;
            }
            if pi < popular.len() {
                let id = &popular[pi];
                if seen.insert(id.clone()) {
                    combined.push(id.clone());
                }
                pi += 1;
            }
        }
        Ok(combined)
    }
}

async fn load_fresh(pg: &PgPool, languages: Option<&[String]>) -> AppResult<Vec<String>> {
    let rows: Vec<String> = if let Some(langs) = languages {
        if !langs.is_empty() {
            sqlx::query_file_scalar!(
                "queries/recommendations/cold_start/fresh_lang.sql",
                langs,
                FRESH_DAYS,
                POOL_FRESH
            )
            .fetch_all(pg)
            .await?
        } else {
            sqlx::query_file_scalar!(
                "queries/recommendations/cold_start/fresh_nolang.sql",
                FRESH_DAYS,
                POOL_FRESH
            )
            .fetch_all(pg)
            .await?
        }
    } else {
        sqlx::query_file_scalar!(
            "queries/recommendations/cold_start/fresh_nolang.sql",
            FRESH_DAYS,
            POOL_FRESH
        )
        .fetch_all(pg)
        .await?
    };
    Ok(rows)
}

async fn load_popular(pg: &PgPool, languages: Option<&[String]>) -> AppResult<Vec<String>> {
    let rows: Vec<String> = if let Some(langs) = languages {
        if !langs.is_empty() {
            sqlx::query_file_scalar!(
                "queries/recommendations/cold_start/popular_lang.sql",
                langs,
                POOL_POPULAR
            )
            .fetch_all(pg)
            .await?
        } else {
            sqlx::query_file_scalar!(
                "queries/recommendations/cold_start/popular_nolang.sql",
                POOL_POPULAR
            )
            .fetch_all(pg)
            .await?
        }
    } else {
        sqlx::query_file_scalar!(
            "queries/recommendations/cold_start/popular_nolang.sql",
            POOL_POPULAR
        )
        .fetch_all(pg)
        .await?
    };
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use sqlx::PgPool;

    async fn seed(pool: &PgPool) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO tracks (
                 sc_track_id, urn, title, title_normalized, duration_ms, sharing, language
             )
             SELECT n::text, 'soundcloud:tracks:' || n, 't' || n, 't' || n, 1000, 'public',
                    (ARRAY['en', 'en', 'en', 'en', 'ru', 'es', 'de', 'fr', 'pt', 'ja'])[1 + n % 10]
             FROM generate_series(1, 20000) AS n",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "INSERT INTO sc_track_counters (sc_track_id, play_count, fetched_at)
             SELECT n::text, n, now() FROM generate_series(1, 20000) AS n",
        )
        .execute(pool)
        .await?;
        sqlx::query("ANALYZE tracks, sc_track_counters")
            .execute(pool)
            .await?;
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_cold_start_pool_reads_the_top_by_index_instead_of_joining_the_catalog(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        seed(&pool).await?;

        let sql = include_str!("../../../queries/recommendations/cold_start/popular_nolang.sql");
        let plan: Value = sqlx::query_scalar(&format!("EXPLAIN (FORMAT JSON) {sql}"))
            .bind(64_i64)
            .fetch_one(&pool)
            .await?;
        let plan = plan.to_string();

        assert!(
            plan.contains("sc_track_counters_play_count_idx"),
            "the cold start pool must walk the play-count index: {plan}"
        );
        assert!(
            !plan.contains("\"Hash Join\"") && !plan.contains("\"Sort\""),
            "the cold start pool must not join and sort the whole catalog to return one page: \
             {plan}"
        );
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_cold_start_pool_still_returns_the_most_played_first(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        seed(&pool).await?;

        let top: Vec<String> = sqlx::query_scalar(include_str!(
            "../../../queries/recommendations/cold_start/popular_nolang.sql"
        ))
        .bind(3_i64)
        .fetch_all(&pool)
        .await?;

        assert_eq!(top, vec!["20000", "19999", "19998"]);
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_language_pool_also_walks_the_play_count_index(pool: PgPool) -> anyhow::Result<()> {
        seed(&pool).await?;

        let sql = include_str!("../../../queries/recommendations/cold_start/popular_lang.sql");
        for languages in [vec!["en".to_owned()], vec!["ja".to_owned()]] {
            let plan: Value = sqlx::query_scalar(&format!("EXPLAIN (FORMAT JSON) {sql}"))
                .bind(&languages)
                .bind(64_i64)
                .fetch_one(&pool)
                .await?;
            let plan = plan.to_string();

            assert!(
                plan.contains("sc_track_counters_play_count_idx"),
                "the language pool must walk the play-count index for {languages:?}: {plan}"
            );
            assert!(
                !plan.contains("\"Hash Join\"") && !plan.contains("\"Sort\""),
                "the language pool must not join and sort the whole catalog for {languages:?}: \
                 {plan}"
            );
        }
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_language_pool_returns_the_most_played_of_that_language(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        seed(&pool).await?;

        let top: Vec<String> = sqlx::query_scalar(include_str!(
            "../../../queries/recommendations/cold_start/popular_lang.sql"
        ))
        .bind(vec!["ja".to_owned()])
        .bind(3_i64)
        .fetch_all(&pool)
        .await?;

        assert_eq!(top, vec!["19999", "19989", "19979"]);
        Ok(())
    }
}
