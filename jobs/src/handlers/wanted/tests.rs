use serde_json::json;
use sqlx::PgPool;

use super::*;

fn wanted(title: &str, artist: &str) -> WantedTrack {
    WantedTrack {
        id: Uuid::now_v7(),
        title: title.to_owned(),
        artist_name: artist.to_owned(),
        duration_ms: Some(180_000),
        isrc: None,
        primary_artist_id: None,
    }
}

fn upload(title: &str, uploader: &str, duration_ms: i64) -> Value {
    json!({
        "urn": "soundcloud:tracks:1",
        "title": title,
        "duration": duration_ms,
        "user": { "username": uploader }
    })
}

#[test]
fn an_exact_upload_from_the_artist_is_linked_outright() {
    let track = wanted("Холод", "Мокери");
    let candidates = vec![upload("Холод", "Мокери", 180_000)];

    let triaged = triage(&candidates, &track, LINK_THRESHOLD);

    assert_eq!(triaged.best.map(|(index, _)| index), Some(0));
    assert!(triaged.borderline.is_empty());
}

#[test]
fn a_reupload_that_names_the_artist_in_the_title_still_matches() {
    let track = wanted("Холод", "Мокери");
    let candidates = vec![upload("Мокери - Холод", "SlowedPhonkArchive", 180_000)];

    let triaged = triage(&candidates, &track, LINK_THRESHOLD);

    assert_eq!(triaged.best.map(|(index, _)| index), Some(0));
}

#[test]
fn an_unrelated_upload_is_neither_linked_nor_offered_to_the_ai() {
    let track = wanted("Холод", "Мокери");
    let candidates = vec![upload("Sunflower", "PostMalone", 158_000)];

    let triaged = triage(&candidates, &track, LINK_THRESHOLD);

    assert!(triaged.best.is_none());
    assert!(triaged.borderline.is_empty());
}

#[test]
fn the_strongest_of_several_uploads_wins() {
    let track = wanted("Холод", "Мокери");
    let candidates = vec![
        upload("Мокери - Холод (slowed)", "SlowedArchive", 240_000),
        upload("Холод", "Мокери", 180_000),
    ];

    let triaged = triage(&candidates, &track, LINK_THRESHOLD);

    assert_eq!(triaged.best.map(|(index, _)| index), Some(1));
}

#[test]
fn a_matching_isrc_links_even_when_the_title_reads_differently() {
    let mut track = wanted("Холод", "Мокери");
    track.isrc = Some("QZES51982531".to_owned());
    let candidates = vec![json!({
        "urn": "soundcloud:tracks:1",
        "title": "kholod (prod. by someone)",
        "duration": 240_000,
        "user": { "username": "Unrelated" },
        "publisher_metadata": { "isrc": "qzes51982531" }
    })];

    let triaged = triage(&candidates, &track, LINK_THRESHOLD);

    assert_eq!(triaged.best.map(|(index, _)| index), Some(0));
}

#[test]
fn an_out_of_range_borderline_index_is_dropped_instead_of_panicking() {
    let candidates = vec![
        upload("Холод", "Мокери", 180_000),
        upload("Холод (slowed)", "Archive", 240_000),
    ];

    let offered = offered_candidates(&candidates, &[1, 7, 0]);

    assert_eq!(
        offered.iter().map(|(index, _)| *index).collect::<Vec<_>>(),
        vec![1, 0]
    );
    assert!(offered_candidates(&[], &[0]).is_empty());
}

async fn install_schema(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE artists (
             id uuid PRIMARY KEY,
             name text NOT NULL
         );
         CREATE TABLE wanted_tracks (
             id uuid PRIMARY KEY,
             title text NOT NULL,
             primary_artist_id uuid REFERENCES artists(id) ON DELETE SET NULL,
             isrc text,
             duration_ms integer,
             status varchar(16) NOT NULL DEFAULT 'wanted',
             track_id uuid,
             resolve_attempts smallint NOT NULL DEFAULT 0,
             resolve_next_run_at timestamptz NOT NULL DEFAULT now(),
             resolve_locked_at timestamptz,
             updated_at timestamptz NOT NULL DEFAULT now()
         );",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn seed_wanted(pool: &PgPool, title: &str) -> anyhow::Result<Uuid> {
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO wanted_tracks (id, title) VALUES ($1, $2)")
        .bind(id)
        .bind(title)
        .execute(pool)
        .await?;
    Ok(id)
}

async fn claim(pool: &PgPool, lease_seconds: f64, batch: i64) -> anyhow::Result<Vec<Uuid>> {
    Ok(
        sqlx::query_file_scalar!("queries/wanted/claim_batch.sql", lease_seconds, batch)
            .fetch_all(pool)
            .await?,
    )
}

async fn state(pool: &PgPool, id: Uuid) -> anyhow::Result<(String, i16, bool)> {
    let row = sqlx::query_as::<_, (String, i16, Option<chrono::DateTime<chrono::Utc>>)>(
        "SELECT status, resolve_attempts, resolve_locked_at FROM wanted_tracks WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await?;
    Ok((row.0, row.1, row.2.is_some()))
}

#[sqlx::test(migrations = false)]
async fn a_claim_leases_the_row_and_counts_the_attempt(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let id = seed_wanted(&pool, "Холод").await?;

    let claimed = claim(&pool, 600.0, 10).await?;

    assert_eq!(claimed, vec![id]);
    assert_eq!(state(&pool, id).await?, ("wanted".to_owned(), 1, true));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_leased_row_is_not_claimed_twice(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_wanted(&pool, "Холод").await?;

    claim(&pool, 600.0, 10).await?;
    let second = claim(&pool, 600.0, 10).await?;

    assert!(second.is_empty());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_expired_lease_is_reclaimed(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let id = seed_wanted(&pool, "Холод").await?;

    claim(&pool, 600.0, 10).await?;
    let reclaimed = claim(&pool, 0.0, 10).await?;

    assert_eq!(reclaimed, vec![id]);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_unresolved_batch_backs_off_and_releases_its_lease(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let id = seed_wanted(&pool, "Холод").await?;
    let claimed = claim(&pool, 600.0, 10).await?;

    sqlx::query_file!("queries/wanted/finalize_backoff.sql", &claimed, 8i16)
        .execute(&pool)
        .await?;
    sqlx::query_file!("queries/wanted/finalize_clear_locks.sql", &claimed)
        .execute(&pool)
        .await?;

    assert_eq!(state(&pool, id).await?, ("wanted".to_owned(), 1, false));
    let due: bool =
        sqlx::query_scalar("SELECT resolve_next_run_at > now() FROM wanted_tracks WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await?;
    assert!(due, "a backed-off row must not be due immediately");
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_wanted_track_retires_at_the_attempt_cap(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let id = seed_wanted(&pool, "Холод").await?;
    sqlx::query("UPDATE wanted_tracks SET resolve_attempts = 7 WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await?;
    let claimed = claim(&pool, 600.0, 10).await?;

    sqlx::query_file!("queries/wanted/finalize_backoff.sql", &claimed, 8i16)
        .execute(&pool)
        .await?;

    assert_eq!(state(&pool, id).await?.0, "unresolvable");
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_unavailable_ai_matcher_never_retires_a_wanted_track(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let id = seed_wanted(&pool, "Холод").await?;
    sqlx::query("UPDATE wanted_tracks SET resolve_attempts = 7 WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await?;

    for _ in 0..3 {
        sqlx::query("UPDATE wanted_tracks SET resolve_next_run_at = now() WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await?;
        let claimed = claim(&pool, 600.0, 10).await?;
        assert_eq!(claimed, vec![id]);
        sqlx::query_file!("queries/wanted/finalize_deferred.sql", &claimed, 1800.0)
            .execute(&pool)
            .await?;
    }

    assert_eq!(state(&pool, id).await?, ("wanted".to_owned(), 7, false));
    let waits: bool = sqlx::query_scalar(
        "SELECT resolve_next_run_at > now() + interval '29 minutes' FROM wanted_tracks WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await?;
    assert!(
        waits,
        "a deferred row waits for the ai lane instead of retrying at once"
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_linked_track_is_left_alone_by_the_backoff(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let id = seed_wanted(&pool, "Холод").await?;
    let claimed = claim(&pool, 600.0, 10).await?;
    sqlx::query("UPDATE wanted_tracks SET status = 'linked', track_id = $1 WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await?;

    sqlx::query_file!("queries/wanted/finalize_backoff.sql", &claimed, 1i16)
        .execute(&pool)
        .await?;

    assert_eq!(state(&pool, id).await?.0, "linked");
    Ok(())
}
