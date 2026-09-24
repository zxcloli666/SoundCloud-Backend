use sqlx::PgPool;

use super::*;

async fn install_schema(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE artists (
             id uuid PRIMARY KEY,
             name text NOT NULL,
             mb_artist_id text,
             genius_artist_id text,
             sc_user_id text,
             has_sc_account boolean NOT NULL DEFAULT false,
             merged_into uuid,
             mb_crawl_offset integer NOT NULL DEFAULT 0,
             genius_crawl_offset integer NOT NULL DEFAULT 0,
             mb_crawled_at timestamptz,
             genius_crawled_at timestamptz,
             mb_next_run_at timestamptz NOT NULL DEFAULT now(),
             genius_next_run_at timestamptz NOT NULL DEFAULT now(),
             mb_locked_at timestamptz,
             genius_locked_at timestamptz,
             crawl_fail_count smallint NOT NULL DEFAULT 0,
             crawl_dead boolean NOT NULL DEFAULT false,
             last_crawled_at timestamptz,
             crawl_attempts smallint NOT NULL DEFAULT 0,
             updated_at timestamptz NOT NULL DEFAULT now()
         );",
    )
    .execute(pool)
    .await?;
    Ok(())
}

struct Seed {
    name: &'static str,
    mb_artist_id: Option<&'static str>,
    genius_artist_id: Option<&'static str>,
    has_sc_account: bool,
}

impl Seed {
    fn artist(name: &'static str) -> Self {
        Self {
            name,
            mb_artist_id: None,
            genius_artist_id: None,
            has_sc_account: false,
        }
    }

    fn on_genius(mut self, id: &'static str) -> Self {
        self.genius_artist_id = Some(id);
        self
    }

    fn on_musicbrainz(mut self, id: &'static str) -> Self {
        self.mb_artist_id = Some(id);
        self
    }

    fn verified_on_soundcloud(mut self) -> Self {
        self.has_sc_account = true;
        self
    }
}

async fn seed(pool: &PgPool, seed: Seed) -> anyhow::Result<Uuid> {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO artists (id, name, mb_artist_id, genius_artist_id, has_sc_account)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(seed.name)
    .bind(seed.mb_artist_id)
    .bind(seed.genius_artist_id)
    .bind(seed.has_sc_account)
    .execute(pool)
    .await?;
    Ok(id)
}

async fn claim_identity(pool: &PgPool) -> anyhow::Result<Vec<Uuid>> {
    let rows = sqlx::query_file!("queries/crawl/claim_identity_lane.sql", 600.0, 10)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|row| row.id).collect())
}

async fn claim_genius(pool: &PgPool) -> anyhow::Result<Vec<Uuid>> {
    let rows = sqlx::query_file!("queries/crawl/claim_genius_lane.sql", 600.0, 10)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|row| row.id).collect())
}

#[sqlx::test(migrations = false)]
async fn a_verified_artist_without_a_genius_id_is_looked_up(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let verified = seed(&pool, Seed::artist("Мокери").verified_on_soundcloud()).await?;
    seed(&pool, Seed::artist("Unknown")).await?;
    seed(
        &pool,
        Seed::artist("Already Known")
            .verified_on_soundcloud()
            .on_genius("42"),
    )
    .await?;

    let claimed = claim_identity(&pool).await?;

    assert_eq!(claimed, vec![verified]);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_matched_identity_becomes_due_for_the_genius_lane(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let artist = seed(&pool, Seed::artist("Мокери").verified_on_soundcloud()).await?;
    claim_identity(&pool).await?;

    sqlx::query_file!("queries/crawl/attach_genius_artist_id.sql", artist, "1312")
        .execute(&pool)
        .await?;

    assert_eq!(claim_genius(&pool).await?, vec![artist]);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_unmatched_identity_is_retried_later_not_forgotten(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let artist = seed(&pool, Seed::artist("Мокери").verified_on_soundcloud()).await?;
    claim_identity(&pool).await?;

    sqlx::query_file!("queries/crawl/defer_identity_lookup.sql", artist, 14.0)
        .execute(&pool)
        .await?;

    assert!(claim_identity(&pool).await?.is_empty());
    let due_within_a_month: bool = sqlx::query_scalar(
        "SELECT genius_next_run_at BETWEEN now() AND now() + interval '30 days'
         FROM artists WHERE id = $1",
    )
    .bind(artist)
    .fetch_one(&pool)
    .await?;
    assert!(due_within_a_month);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn the_musicbrainz_lane_leaves_genius_artists_to_the_genius_lane(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let mb_only = seed(&pool, Seed::artist("MB Only").on_musicbrainz("mbid-1")).await?;
    seed(
        &pool,
        Seed::artist("Both").on_musicbrainz("mbid-2").on_genius("7"),
    )
    .await?;

    let rows = sqlx::query_file!("queries/crawl/claim_mb_lane.sql", 600.0, 10)
        .fetch_all(&pool)
        .await?;

    assert_eq!(
        rows.into_iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![mb_only]
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_failing_artist_backs_off_before_it_is_declared_dead(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let artist = seed(&pool, Seed::artist("Мокери").on_genius("1312")).await?;
    claim_genius(&pool).await?;

    sqlx::query_file!(
        "queries/crawl/lane_backoff_genius.sql",
        artist,
        3i16,
        next_run_after(3)
    )
    .execute(&pool)
    .await?;

    assert!(claim_genius(&pool).await?.is_empty());
    let dead: bool = sqlx::query_scalar("SELECT crawl_dead FROM artists WHERE id = $1")
        .bind(artist)
        .fetch_one(&pool)
        .await?;
    assert!(!dead);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_dead_artist_is_never_claimed_again(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let artist = seed(&pool, Seed::artist("Мокери").on_genius("1312")).await?;
    claim_genius(&pool).await?;

    sqlx::query_file!("queries/crawl/lane_dead_genius.sql", artist, 8i16)
        .execute(&pool)
        .await?;

    assert!(claim_genius(&pool).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_successful_genius_crawl_refreshes_both_cursors(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let artist = seed(
        &pool,
        Seed::artist("Мокери")
            .on_genius("1312")
            .on_musicbrainz("mbid-1"),
    )
    .await?;
    claim_genius(&pool).await?;

    sqlx::query_file!("queries/crawl/lane_success_genius.sql", artist, 14.0)
        .execute(&pool)
        .await?;

    let (genius_crawled, mb_crawled): (bool, bool) = sqlx::query_as(
        "SELECT genius_crawled_at IS NOT NULL, mb_crawled_at IS NOT NULL
         FROM artists WHERE id = $1",
    )
    .bind(artist)
    .fetch_one(&pool)
    .await?;

    assert!(genius_crawled);
    assert!(mb_crawled);
    Ok(())
}
