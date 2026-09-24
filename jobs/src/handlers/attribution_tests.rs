use super::*;

async fn install_schema(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE artists (
             id uuid PRIMARY KEY,
             name text NOT NULL
         );
         CREATE TABLE artist_sc_accounts (
             artist_id uuid NOT NULL REFERENCES artists(id) ON DELETE CASCADE,
             sc_user_id text NOT NULL,
             role varchar(8) NOT NULL DEFAULT 'main',
             source varchar(16) NOT NULL,
             verified boolean NOT NULL DEFAULT false,
             PRIMARY KEY (artist_id, sc_user_id)
         );
         CREATE TABLE tracks (
             id uuid PRIMARY KEY,
             uploader_sc_user_id text,
             primary_artist_id uuid REFERENCES artists(id) ON DELETE SET NULL,
             enrich_state varchar(16) NOT NULL DEFAULT 'done',
             enrich_source varchar(16),
             enrich_confidence real,
             enrich_error text,
             enrich_attempts smallint NOT NULL DEFAULT 0,
             enrich_locked_at timestamptz,
             enrich_next_run_at timestamptz,
             updated_at timestamptz NOT NULL DEFAULT now()
         );
         CREATE TABLE track_artists (
             track_id uuid NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
             artist_id uuid NOT NULL REFERENCES artists(id) ON DELETE CASCADE,
             role varchar(16) NOT NULL,
             position smallint NOT NULL DEFAULT 0,
             source varchar(16) NOT NULL,
             confidence real NOT NULL DEFAULT 0,
             PRIMARY KEY (track_id, artist_id, role)
         );
         CREATE TABLE artist_attribution_revalidation (
             singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
             walker_completed_at timestamptz,
             identity_cursor_artist uuid,
             identity_cursor_account text,
             identity_completed_at timestamptz,
             credits_removed bigint NOT NULL DEFAULT 0,
             tracks_reset bigint NOT NULL DEFAULT 0,
             updated_at timestamptz
         );
         INSERT INTO artist_attribution_revalidation (singleton) VALUES (true);",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn seed_artist(pool: &PgPool, name: &str) -> anyhow::Result<Uuid> {
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO artists (id, name) VALUES ($1, $2)")
        .bind(id)
        .bind(name)
        .execute(pool)
        .await?;
    Ok(id)
}

async fn seed_track(
    pool: &PgPool,
    uploader: &str,
    artist_id: Uuid,
    credit_source: &str,
    enrich_source: &str,
) -> anyhow::Result<Uuid> {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO tracks (id, uploader_sc_user_id, primary_artist_id, enrich_state, enrich_source)
         VALUES ($1, $2, $3, 'done', $4)",
    )
    .bind(id)
    .bind(uploader)
    .bind(artist_id)
    .bind(enrich_source)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO track_artists (track_id, artist_id, role, source, confidence)
         VALUES ($1, $2, 'primary', $3, 0.85)",
    )
    .bind(id)
    .bind(artist_id)
    .bind(credit_source)
    .execute(pool)
    .await?;
    Ok(id)
}

async fn state_of(pool: &PgPool, track_id: Uuid) -> anyhow::Result<(Option<Uuid>, String, i64)> {
    let row = sqlx::query_as::<_, (Option<Uuid>, String, i64)>(
        "SELECT track.primary_artist_id,
                track.enrich_state,
                (SELECT count(*) FROM track_artists AS credit WHERE credit.track_id = track.id)
         FROM tracks AS track
         WHERE track.id = $1",
    )
    .bind(track_id)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

#[sqlx::test(migrations = false)]
async fn walker_credits_are_removed_and_the_phase_completes(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let artist = seed_artist(&pool, "Zemix").await?;
    let walked = seed_track(&pool, "10", artist, "walker", "heuristic").await?;
    let resolved = seed_track(&pool, "11", artist, "genius", "genius").await?;
    let handler = AttributionHandler::new(pool.clone());

    handler
        .revalidate()
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    assert_eq!(
        state_of(&pool, walked).await?,
        (None, "pending".to_owned(), 0)
    );
    assert_eq!(
        state_of(&pool, resolved).await?,
        (Some(artist), "done".to_owned(), 1)
    );

    handler
        .revalidate()
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    let completed = sqlx::query_scalar::<_, bool>(
        "SELECT walker_completed_at IS NOT NULL FROM artist_attribution_revalidation",
    )
    .fetch_one(&pool)
    .await?;
    assert!(completed);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn locked_tracks_keep_their_credits_until_enrichment_releases_them(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let artist = seed_artist(&pool, "Zemix").await?;
    let locked = seed_track(&pool, "10", artist, "walker", "heuristic").await?;
    sqlx::query("UPDATE tracks SET enrich_locked_at = now() WHERE id = $1")
        .bind(locked)
        .execute(&pool)
        .await?;

    AttributionHandler::new(pool.clone())
        .revalidate()
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    assert_eq!(
        state_of(&pool, locked).await?,
        (Some(artist), "done".to_owned(), 1)
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn unverified_account_claims_are_returned_to_enrichment(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    sqlx::query("UPDATE artist_attribution_revalidation SET walker_completed_at = now()")
        .execute(&pool)
        .await?;
    let artist = seed_artist(&pool, "Psychosis").await?;
    sqlx::query(
        "INSERT INTO artist_sc_accounts (artist_id, sc_user_id, role, source, verified)
         VALUES ($1, '77', 'alt', 'reupload_pattern', false)",
    )
    .bind(artist)
    .execute(&pool)
    .await?;
    let claimed = seed_track(&pool, "77", artist, "sc_verified", "sc_verified").await?;
    let resolved = seed_track(&pool, "77", artist, "genius", "genius").await?;
    let handler = AttributionHandler::new(pool.clone());

    handler
        .revalidate()
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    assert_eq!(
        state_of(&pool, claimed).await?,
        (None, "pending".to_owned(), 0)
    );
    assert_eq!(
        state_of(&pool, resolved).await?,
        (Some(artist), "done".to_owned(), 1)
    );

    handler
        .revalidate()
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    let completed = sqlx::query_scalar::<_, bool>(
        "SELECT identity_completed_at IS NOT NULL FROM artist_attribution_revalidation",
    )
    .fetch_one(&pool)
    .await?;
    assert!(completed);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn verified_account_claims_survive(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    sqlx::query("UPDATE artist_attribution_revalidation SET walker_completed_at = now()")
        .execute(&pool)
        .await?;
    let artist = seed_artist(&pool, "dekma").await?;
    sqlx::query(
        "INSERT INTO artist_sc_accounts (artist_id, sc_user_id, role, source, verified)
         VALUES ($1, '5', 'main', 'manual', true)",
    )
    .bind(artist)
    .execute(&pool)
    .await?;
    let owned = seed_track(&pool, "5", artist, "sc_verified", "sc_verified").await?;

    AttributionHandler::new(pool.clone())
        .revalidate()
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    assert_eq!(
        state_of(&pool, owned).await?,
        (Some(artist), "done".to_owned(), 1)
    );
    Ok(())
}
