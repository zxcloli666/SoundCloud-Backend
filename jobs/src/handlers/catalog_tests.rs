use super::*;

async fn install_schema(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE artists (
             id uuid PRIMARY KEY,
             name text NOT NULL
         );
         CREATE TABLE tracks (
             id uuid PRIMARY KEY,
             title text NOT NULL,
             duration_ms integer,
             primary_artist_id uuid REFERENCES artists(id) ON DELETE SET NULL,
             work_key text,
             recording_key text,
             work_normalizer_version smallint,
             canonical_track_id uuid,
             superseded_by uuid REFERENCES tracks(id) ON DELETE SET NULL,
             storage_state varchar(16) NOT NULL DEFAULT 'pending',
             index_state varchar(16) NOT NULL DEFAULT 'pending',
             s3_verified_at timestamptz,
             quality_score real,
             play_count_sc bigint,
             created_at timestamptz NOT NULL DEFAULT now(),
             updated_at timestamptz NOT NULL DEFAULT now()
         );
         CREATE TABLE track_work_aliases (
             track_id uuid NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
             alias_key text NOT NULL,
             PRIMARY KEY (track_id, alias_key)
         );
         CREATE TABLE wanted_tracks (
             id uuid PRIMARY KEY,
             title text NOT NULL,
             duration_ms integer,
             primary_artist_id uuid REFERENCES artists(id) ON DELETE SET NULL,
             status varchar(16) NOT NULL DEFAULT 'wanted',
             track_id uuid REFERENCES tracks(id) ON DELETE SET NULL,
             work_key text,
             recording_key text,
             work_normalizer_version smallint,
             work_reconciled_at timestamptz,
             updated_at timestamptz NOT NULL DEFAULT now()
         );
         CREATE TABLE wanted_track_work_aliases (
             wanted_track_id uuid NOT NULL REFERENCES wanted_tracks(id) ON DELETE CASCADE,
             alias_key text NOT NULL,
             PRIMARY KEY (wanted_track_id, alias_key)
         );
         CREATE TABLE catalog_merge_state (
             singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
             cursor_artist uuid,
             cursor_recording_key text,
             passes_completed bigint NOT NULL DEFAULT 0,
             groups_merged bigint NOT NULL DEFAULT 0,
             tracks_superseded bigint NOT NULL DEFAULT 0,
             updated_at timestamptz
         );
         INSERT INTO catalog_merge_state (singleton) VALUES (true);
         CREATE TABLE catalog_work_links (
             wanted_track_id uuid NOT NULL REFERENCES wanted_tracks(id) ON DELETE CASCADE,
             track_id uuid NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
             match_reason varchar(32) NOT NULL,
             matched_key text NOT NULL,
             normalizer_version smallint NOT NULL,
             linked_at timestamptz NOT NULL DEFAULT now(),
             PRIMARY KEY (wanted_track_id, track_id)
         );",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn seed_artist(pool: &PgPool) -> anyhow::Result<Uuid> {
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO artists (id, name) VALUES ($1, 'Мокери')")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(id)
}

async fn seed_track(
    pool: &PgPool,
    artist: Uuid,
    title: &str,
    duration_ms: Option<i32>,
) -> anyhow::Result<Uuid> {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO tracks (id, title, duration_ms, primary_artist_id) VALUES ($1, $2, $3, $4)",
    )
    .bind(id)
    .bind(title)
    .bind(duration_ms)
    .bind(artist)
    .execute(pool)
    .await?;
    Ok(id)
}

async fn seed_wanted(
    pool: &PgPool,
    artist: Uuid,
    title: &str,
    duration_ms: Option<i32>,
) -> anyhow::Result<Uuid> {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO wanted_tracks (id, title, duration_ms, primary_artist_id) VALUES ($1, $2, $3, $4)",
    )
    .bind(id)
    .bind(title)
    .bind(duration_ms)
    .bind(artist)
    .execute(pool)
    .await?;
    Ok(id)
}

async fn linked_track(pool: &PgPool, wanted: Uuid) -> anyhow::Result<Option<Uuid>> {
    let row =
        sqlx::query_scalar::<_, Option<Uuid>>("SELECT track_id FROM wanted_tracks WHERE id = $1")
            .bind(wanted)
            .fetch_one(pool)
            .await?;
    Ok(row)
}

async fn run(pool: &PgPool) -> anyhow::Result<()> {
    CatalogWorkHandler::new(pool.clone())
        .reconcile()
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))
}

#[sqlx::test(migrations = false)]
async fn translated_title_links_to_the_local_upload(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let artist = seed_artist(&pool).await?;
    let track = seed_track(&pool, artist, "холод (cold)", Some(180_000)).await?;
    let wanted = seed_wanted(&pool, artist, "Холод", None).await?;

    run(&pool).await?;

    assert_eq!(linked_track(&pool, wanted).await?, Some(track));
    let reason = sqlx::query_scalar::<_, String>(
        "SELECT match_reason FROM catalog_work_links WHERE wanted_track_id = $1",
    )
    .bind(wanted)
    .fetch_one(&pool)
    .await?;
    assert_eq!(reason, "recording");
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn transliterated_genius_title_links_through_an_alias(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let artist = seed_artist(&pool).await?;
    let track = seed_track(&pool, artist, "Холод", Some(180_000)).await?;
    let wanted = seed_wanted(&pool, artist, "Kholod", Some(181_000)).await?;

    run(&pool).await?;

    assert_eq!(linked_track(&pool, wanted).await?, Some(track));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_different_duration_is_not_the_same_recording(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let artist = seed_artist(&pool).await?;
    seed_track(&pool, artist, "Холод", Some(120_000)).await?;
    let wanted = seed_wanted(&pool, artist, "Холод", Some(320_000)).await?;

    run(&pool).await?;

    assert_eq!(linked_track(&pool, wanted).await?, None);
    let stamped = sqlx::query_scalar::<_, bool>(
        "SELECT work_reconciled_at IS NOT NULL FROM wanted_tracks WHERE id = $1",
    )
    .bind(wanted)
    .fetch_one(&pool)
    .await?;
    assert!(stamped);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn another_artist_with_the_same_title_is_never_linked(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let ours = seed_artist(&pool).await?;
    let other = Uuid::now_v7();
    sqlx::query("INSERT INTO artists (id, name) VALUES ($1, 'Someone Else')")
        .bind(other)
        .execute(&pool)
        .await?;
    seed_track(&pool, other, "Холод", Some(180_000)).await?;
    let wanted = seed_wanted(&pool, ours, "Холод", Some(180_000)).await?;

    run(&pool).await?;

    assert_eq!(linked_track(&pool, wanted).await?, None);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn the_exact_version_wins_over_a_remix_of_the_same_work(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let artist = seed_artist(&pool).await?;
    let remix = seed_track(&pool, artist, "Холод (Skrillex Remix)", Some(180_000)).await?;
    let original = seed_track(&pool, artist, "Холод", Some(180_000)).await?;
    let wanted = seed_wanted(&pool, artist, "Холод", Some(180_000)).await?;

    run(&pool).await?;

    assert_eq!(linked_track(&pool, wanted).await?, Some(original));
    assert_ne!(linked_track(&pool, wanted).await?, Some(remix));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn keys_are_written_once_and_aliases_follow_the_title(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let artist = seed_artist(&pool).await?;
    let track = seed_track(&pool, artist, "холод (cold)", Some(180_000)).await?;

    run(&pool).await?;

    let keys = sqlx::query_as::<_, (Option<String>, Option<String>, Option<i16>)>(
        "SELECT work_key, recording_key, work_normalizer_version FROM tracks WHERE id = $1",
    )
    .bind(track)
    .fetch_one(&pool)
    .await?;
    assert_eq!(keys.0.as_deref(), Some("холод"));
    assert_eq!(keys.1.as_deref(), Some("холод"));
    assert_eq!(keys.2, Some(NORMALIZER_VERSION));

    let aliases = sqlx::query_scalar::<_, String>(
        "SELECT alias_key FROM track_work_aliases WHERE track_id = $1 ORDER BY alias_key",
    )
    .bind(track)
    .fetch_all(&pool)
    .await?;
    assert_eq!(aliases, vec!["cold".to_owned(), "kholod".to_owned()]);
    Ok(())
}

async fn seed_playable(
    pool: &PgPool,
    artist: Uuid,
    title: &str,
    quality: f32,
) -> anyhow::Result<Uuid> {
    let id = seed_track(pool, artist, title, Some(180_000)).await?;
    sqlx::query(
        "UPDATE tracks
         SET storage_state = 'ok',
             index_state = 'ok',
             s3_verified_at = now(),
             quality_score = $2
         WHERE id = $1",
    )
    .bind(id)
    .bind(quality)
    .execute(pool)
    .await?;
    Ok(id)
}

async fn superseded_by(pool: &PgPool, track: Uuid) -> anyhow::Result<Option<Uuid>> {
    let row =
        sqlx::query_scalar::<_, Option<Uuid>>("SELECT superseded_by FROM tracks WHERE id = $1")
            .bind(track)
            .fetch_one(pool)
            .await?;
    Ok(row)
}

#[sqlx::test(migrations = false)]
async fn the_playable_upload_wins_over_the_silent_duplicate(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let artist = seed_artist(&pool).await?;
    let broken = seed_track(&pool, artist, "Холод", Some(180_000)).await?;
    let playable = seed_playable(&pool, artist, "холод (cold)", 0.4).await?;

    run(&pool).await?;

    assert_eq!(superseded_by(&pool, broken).await?, Some(playable));
    assert_eq!(superseded_by(&pool, playable).await?, None);
    let group = sqlx::query_scalar::<_, i64>(
        "SELECT count(DISTINCT canonical_track_id) FROM tracks WHERE id IN ($1, $2)",
    )
    .bind(broken)
    .bind(playable)
    .fetch_one(&pool)
    .await?;
    assert_eq!(group, 1);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn the_higher_ranked_upload_wins_between_two_playable_duplicates(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let artist = seed_artist(&pool).await?;
    let weak = seed_playable(&pool, artist, "Холод", 0.2).await?;
    let strong = seed_playable(&pool, artist, "Холод", 0.9).await?;

    run(&pool).await?;

    assert_eq!(superseded_by(&pool, weak).await?, Some(strong));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn different_versions_of_one_work_are_never_merged(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let artist = seed_artist(&pool).await?;
    let original = seed_playable(&pool, artist, "Холод", 0.5).await?;
    let sped_up = seed_playable(&pool, artist, "Холод (sped up)", 0.5).await?;

    run(&pool).await?;

    assert_eq!(superseded_by(&pool, original).await?, None);
    assert_eq!(superseded_by(&pool, sped_up).await?, None);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_longer_recording_is_not_folded_into_a_short_one(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let artist = seed_artist(&pool).await?;
    let short = seed_playable(&pool, artist, "Холод", 0.9).await?;
    let long = seed_track(&pool, artist, "Холод", Some(400_000)).await?;

    run(&pool).await?;

    assert_eq!(superseded_by(&pool, long).await?, None);
    assert_eq!(superseded_by(&pool, short).await?, None);
    Ok(())
}
