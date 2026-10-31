use serde_json::json;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use url::Url;
use uuid::Uuid;
use wt::Kind;

use super::model::{Authority, PlaylistSnapshot};
use super::repository::{ObservationCapture, PlaylistObserveRepository};
use super::urn::PlaylistUrn;

const PLAYLIST: &str = "soundcloud:playlists:77";
const OWNER: &str = "17";
const STATE: &str = "playlist_membership_state";
const PLAYLISTS: &str = "playlists";
const ROUNDS: usize = 200;

async fn seed(pool: &PgPool, tracks: usize) -> anyhow::Result<()> {
    crate::db::migrations::run_core(pool, None).await?;
    sqlx::query(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, uploader_sc_user_id)
         SELECT n::text, 'soundcloud:tracks:' || n, 'Track', 'track', 120000, $1
         FROM generate_series(1, $2) AS n",
    )
    .bind(OWNER)
    .bind(i32::try_from(tracks)?)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO playlists (sc_playlist_id, urn, title, title_normalized, owner_sc_user_id, track_count, sharing)
         VALUES ('77', $1, 'Mix', 'mix', $2, 2, 'public')",
    )
    .bind(PLAYLIST)
    .bind(OWNER)
    .execute(pool)
    .await?;
    sqlx::query("INSERT INTO user_owned_playlists (user_id, playlist_urn) VALUES ($1, $2)")
        .bind(OWNER)
        .bind(PLAYLIST)
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO playlist_membership_state
             (playlist_urn, owner_sc_user_id, projection_track_count, sync_status, next_reconcile_at)
         VALUES ($1, $2, 2, 'clean', clock_timestamp())",
    )
    .bind(PLAYLIST)
    .bind(OWNER)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO playlist_track_projection (playlist_urn, position, sc_track_id)
         VALUES ($1, 0, '1'), ($1, 1, '2')",
    )
    .bind(PLAYLIST)
    .execute(pool)
    .await?;
    Ok(())
}

async fn record_writes(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE lock_order_writes (n bigserial PRIMARY KEY, tbl text NOT NULL);
         CREATE FUNCTION lock_order_write() RETURNS trigger LANGUAGE plpgsql AS $f$
         BEGIN
             INSERT INTO lock_order_writes (tbl) VALUES (TG_TABLE_NAME);
             RETURN NULL;
         END
         $f$;
         CREATE TRIGGER lock_order_write AFTER INSERT OR UPDATE OR DELETE ON playlist_membership_state
             FOR EACH ROW EXECUTE FUNCTION lock_order_write();
         CREATE TRIGGER lock_order_write AFTER INSERT OR UPDATE OR DELETE ON playlists
             FOR EACH ROW EXECUTE FUNCTION lock_order_write();
         CREATE TRIGGER lock_order_write AFTER INSERT OR UPDATE OR DELETE ON playlist_track_projection
             FOR EACH ROW EXECUTE FUNCTION lock_order_write();
         CREATE TRIGGER lock_order_write AFTER INSERT OR UPDATE OR DELETE ON playlist_membership_operations
             FOR EACH ROW EXECUTE FUNCTION lock_order_write();
         CREATE TRIGGER lock_order_write AFTER INSERT OR UPDATE OR DELETE ON tracks
             FOR EACH ROW EXECUTE FUNCTION lock_order_write();
         CREATE TRIGGER lock_order_write AFTER INSERT OR UPDATE OR DELETE ON sync_queue
             FOR EACH ROW EXECUTE FUNCTION lock_order_write();
         CREATE TRIGGER lock_order_write AFTER INSERT OR UPDATE OR DELETE ON user_owned_playlists
             FOR EACH ROW EXECUTE FUNCTION lock_order_write();",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn writes_since(pool: &PgPool, start: i64) -> anyhow::Result<Vec<String>> {
    let mut tables: Vec<String> =
        sqlx::query_scalar("SELECT tbl FROM lock_order_writes WHERE n > $1 ORDER BY n")
            .bind(start)
            .fetch_all(pool)
            .await?;
    tables.dedup();
    Ok(tables)
}

async fn last_write(pool: &PgPool) -> anyhow::Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT coalesce(max(n), 0) FROM lock_order_writes")
            .fetch_one(pool)
            .await?,
    )
}

fn assert_state_then_playlist(writes: &[String]) {
    assert_eq!(&writes[..2], [STATE, PLAYLISTS], "{writes:?}");
    assert!(
        !writes[2..].iter().any(|table| table == PLAYLISTS),
        "{writes:?}"
    );
}

async fn capture(
    repository: &PlaylistObserveRepository,
) -> anyhow::Result<Option<ObservationCapture>> {
    Ok(repository
        .capture(&PlaylistUrn::parse(PLAYLIST)?, Uuid::now_v7(), 1)
        .await?
        .capture)
}

async fn projection(pool: &PgPool) -> anyhow::Result<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT sc_track_id FROM playlist_track_projection WHERE playlist_urn = $1 ORDER BY position",
    )
    .bind(PLAYLIST)
    .fetch_all(pool)
    .await?)
}

fn snapshot(track_ids: Vec<String>) -> PlaylistSnapshot {
    PlaylistSnapshot {
        playlist_id: "77".to_owned(),
        owner_id: OWNER.to_owned(),
        track_count: i32::try_from(track_ids.len()).unwrap_or(i32::MAX),
        track_ids,
        hydrated_tracks: Vec::new(),
        remote_last_modified: chrono::Utc::now(),
        observed_at: chrono::Utc::now(),
    }
}

#[sqlx::test(migrations = false)]
async fn persist_writes_the_state_row_before_the_playlist_and_the_projection(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool, 3).await?;
    record_writes(&pool).await?;
    let repository = PlaylistObserveRepository::new(pool.clone(), false);
    let captured = capture(&repository)
        .await?
        .ok_or_else(|| anyhow::anyhow!("capture"))?;
    let start = last_write(&pool).await?;
    repository
        .persist_success(
            &captured,
            &snapshot(vec!["1".into(), "2".into(), "3".into()]),
            Authority::Owner,
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;
    assert_state_then_playlist(&writes_since(&pool, start).await?);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_remote_delete_writes_the_state_row_before_the_playlist(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool, 2).await?;
    record_writes(&pool).await?;
    let repository = PlaylistObserveRepository::new(pool.clone(), false);
    let captured = capture(&repository)
        .await?
        .ok_or_else(|| anyhow::anyhow!("capture"))?;
    let start = last_write(&pool).await?;
    assert!(repository.retire_remotely_deleted(&captured).await?);
    assert_state_then_playlist(&writes_since(&pool, start).await?);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_observed_playlist_writes_the_state_row_before_the_playlist(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool, 2).await?;
    record_writes(&pool).await?;
    let start = last_write(&pool).await?;
    let remote = json!({
        "urn": PLAYLIST,
        "title": "Renamed",
        "sharing": "public",
        "track_count": 2,
        "user": {"urn": "soundcloud:users:17"}
    });
    catalog_ingest::upsert_playlist_from_sc(
        &pool,
        &remote,
        catalog_ingest::Observation::begin(&pool).await?,
    )
    .await?;
    assert_state_then_playlist(&writes_since(&pool, start).await?);
    Ok(())
}

async fn split_core(pool: &PgPool) -> anyhow::Result<PgPool> {
    sqlx::query("SELECT count(*) FROM public.scd_configure('core', 'core', 'split')")
        .execute(pool)
        .await?;
    let database: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(pool)
        .await?;
    let mut url = Url::parse(&std::env::var("DATABASE_URL")?)?;
    url.set_path(&database);
    Ok(PgPoolOptions::new()
        .max_connections(8)
        .connect(url.as_str())
        .await?)
}

fn add(track: usize) -> wt::MembershipOp {
    let key = Uuid::new_v4();
    wt::MembershipOp {
        playlist_urn: PLAYLIST.to_owned(),
        actor: OWNER.to_owned(),
        variants: catalog_ingest::user_id_variants(OWNER),
        expected_projection_revision: None,
        operations: vec![wt::MembershipOperation {
            operation_id: Uuid::new_v4(),
            idempotency_key: key,
            fingerprint: format!("{0}{0}", key.simple()),
            kind: "add".to_owned(),
            track_id: Some(track.to_string()),
            left_anchor_track_id: None,
            right_anchor_track_id: None,
            boundary: Some("back".to_owned()),
            ordered_track_ids: None,
        }],
        at: wt::now(),
    }
}

async fn dispatch_from_main(core: &PgPool, command: &wt::MembershipOp) -> anyhow::Result<()> {
    let mut transaction = core.begin().await?;
    sqlx::query("SET LOCAL session_replication_role = replica")
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "INSERT INTO cmd_outbox (node, id, kind, args, touched) VALUES ('main', $1, $2, $3, '[]'::jsonb)",
    )
    .bind(Uuid::new_v4())
    .bind(wt::MembershipOp::NAME)
    .bind(serde_json::to_value(command)?)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(())
}

async fn observe_round(
    core: &PgPool,
    repository: &PlaylistObserveRepository,
) -> anyhow::Result<()> {
    let Some(captured) = capture(repository).await? else {
        return Ok(());
    };
    let remote = snapshot(projection(core).await?);
    repository
        .persist_success(
            &captured,
            &remote,
            Authority::Owner,
            catalog_ingest::Observation::begin(core).await?,
        )
        .await?;
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn membership_ops_dispatched_on_core_and_concurrent_persists_never_deadlock(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool, ROUNDS + 2).await?;
    let core = split_core(&pool).await?;
    let repository = PlaylistObserveRepository::new(core.clone(), false);
    let mut failures = Vec::new();
    let dispatched = async {
        let mut failed = Vec::new();
        for round in 0..ROUNDS {
            if let Err(error) = dispatch_from_main(&core, &add(round + 3)).await {
                failed.push(format!("dispatch {round}: {error}"));
            }
        }
        failed
    };
    let observed = async {
        let mut failed = Vec::new();
        for round in 0..ROUNDS {
            if let Err(error) = observe_round(&core, &repository).await {
                failed.push(format!("persist {round}: {error}"));
            }
        }
        failed
    };
    let (dispatch_failures, persist_failures) = tokio::join!(dispatched, observed);
    failures.extend(dispatch_failures);
    failures.extend(persist_failures);
    assert!(failures.is_empty(), "{failures:#?}");

    let commands: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM cmd_done WHERE node = 'main'), (SELECT count(*) FROM cmd_dead)",
    )
    .fetch_one(&core)
    .await?;
    assert_eq!(commands, (i64::try_from(ROUNDS)?, 0));
    let journal: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT count(*), count(DISTINCT idempotency_key), count(*) FILTER (WHERE outcome = 'conflict'),
                (SELECT last_operation_sequence FROM playlist_membership_state WHERE playlist_urn = $1)
         FROM playlist_membership_operations WHERE playlist_urn = $1",
    )
    .bind(PLAYLIST)
    .fetch_one(&core)
    .await?;
    let rounds = i64::try_from(ROUNDS)?;
    assert_eq!(journal, (rounds, rounds, 0, rounds));
    let expected: Vec<String> = (1..=ROUNDS + 2).map(|track| track.to_string()).collect();
    assert_eq!(projection(&core).await?, expected);
    Ok(())
}
