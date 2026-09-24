use sqlx::PgPool;
use uuid::Uuid;

use super::edit::{EditBody, MembershipRequest, MoveBody, TrackEdit};
use super::journal::{
    AWAITING_BASELINE, LEGACY_RECONCILIATION_PENDING, PlaylistJournal, REVISION_CONFLICT,
    UNKNOWN_TRACK,
};
use super::test_schema;

const PLAYLIST: &str = "soundcloud:playlists:42";
const OWNER: &str = "17";

fn body(json: serde_json::Value) -> MembershipRequest {
    serde_json::from_value::<EditBody>(json)
        .expect("edit body")
        .into_request()
        .expect("membership request")
}

async fn projection(pool: &PgPool) -> anyhow::Result<Vec<String>> {
    let ids = sqlx::query_scalar::<_, String>(
        "SELECT sc_track_id FROM playlist_track_projection
         WHERE playlist_urn = $1 ORDER BY position",
    )
    .bind(PLAYLIST)
    .fetch_all(pool)
    .await?;
    Ok(ids)
}

async fn journalled(pool: &PgPool) -> anyhow::Result<Vec<(i64, String, Option<String>)>> {
    let rows = sqlx::query_as::<_, (i64, String, Option<String>)>(
        "SELECT sequence, kind, track_id FROM playlist_membership_operations
         WHERE playlist_urn = $1 ORDER BY sequence",
    )
    .bind(PLAYLIST)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

async fn state(pool: &PgPool) -> anyhow::Result<(i64, i64, i32, String)> {
    let row = sqlx::query_as::<_, (i64, i64, i32, String)>(
        "SELECT last_operation_sequence, projection_revision, projection_track_count, sync_status
         FROM playlist_membership_state WHERE playlist_urn = $1",
    )
    .bind(PLAYLIST)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

#[sqlx::test(migrations = false)]
async fn a_pending_local_edit_does_not_defeat_the_observation_throttle(
    pool: PgPool,
) -> anyhow::Result<()> {
    test_schema::install(&pool).await?;
    test_schema::seed_playlist(&pool, PLAYLIST, OWNER, &["1", "2", "3"]).await?;
    let journal = PlaylistJournal::new(pool.clone());

    journal
        .append(
            PLAYLIST,
            OWNER,
            body(serde_json::json!({ "remove": "soundcloud:tracks:3" })),
            Uuid::now_v7(),
        )
        .await?;
    sqlx::query(
        "UPDATE playlist_membership_state
         SET next_reconcile_at = clock_timestamp() + interval '1 hour'
         WHERE playlist_urn = $1",
    )
    .bind(PLAYLIST)
    .execute(&pool)
    .await?;
    sqlx::query_file!(
        "../utils/catalog-ingest/queries/playlists/ensure_membership_state.sql",
        PLAYLIST
    )
    .execute(&pool)
    .await?;

    let due: bool = sqlx::query_scalar(
        "SELECT next_reconcile_at <= clock_timestamp()
         FROM playlist_membership_state WHERE playlist_urn = $1",
    )
    .bind(PLAYLIST)
    .fetch_one(&pool)
    .await?;
    assert!(!due);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_addition_is_journalled_and_visible_in_the_projection(
    pool: PgPool,
) -> anyhow::Result<()> {
    test_schema::install(&pool).await?;
    test_schema::seed_playlist(&pool, PLAYLIST, OWNER, &["1", "2", "9"]).await?;
    let journal = PlaylistJournal::new(pool.clone());

    let outcome = journal
        .append(
            PLAYLIST,
            OWNER,
            body(serde_json::json!({ "add": "soundcloud:tracks:9" })),
            Uuid::now_v7(),
        )
        .await?;
    assert_eq!(outcome.appended, 0);

    sqlx::query("DELETE FROM playlist_track_projection WHERE sc_track_id = '9'")
        .execute(&pool)
        .await?;
    let outcome = journal
        .append(
            PLAYLIST,
            OWNER,
            body(serde_json::json!({ "add": "soundcloud:tracks:9" })),
            Uuid::now_v7(),
        )
        .await?;

    assert_eq!(outcome.appended, 1);
    assert_eq!(projection(&pool).await?, vec!["1", "2", "9"]);
    assert_eq!(
        journalled(&pool).await?,
        vec![(1, "add".to_owned(), Some("9".to_owned()))]
    );
    assert_eq!(state(&pool).await?, (1, 1, 3, "pending".to_owned()));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_repeated_request_with_the_same_idempotency_key_is_applied_once(
    pool: PgPool,
) -> anyhow::Result<()> {
    test_schema::install(&pool).await?;
    test_schema::seed_playlist(&pool, PLAYLIST, OWNER, &["1", "2"]).await?;
    sqlx::query("INSERT INTO tracks (sc_track_id, urn, title) VALUES ('9', 'x', 'x')")
        .execute(&pool)
        .await?;
    let journal = PlaylistJournal::new(pool.clone());
    let key = Uuid::now_v7();

    journal
        .append(
            PLAYLIST,
            OWNER,
            body(serde_json::json!({ "add": "soundcloud:tracks:9" })),
            key,
        )
        .await?;
    sqlx::query("DELETE FROM playlist_track_projection WHERE sc_track_id = '9'")
        .execute(&pool)
        .await?;
    let replay = journal
        .append(
            PLAYLIST,
            OWNER,
            body(serde_json::json!({ "add": "soundcloud:tracks:9" })),
            key,
        )
        .await?;

    assert_eq!(replay.appended, 0);
    assert_eq!(journalled(&pool).await?.len(), 1);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_append_and_a_removal_touch_only_their_own_projection_rows(
    pool: PgPool,
) -> anyhow::Result<()> {
    test_schema::install(&pool).await?;
    test_schema::seed_playlist(&pool, PLAYLIST, OWNER, &["1", "2", "3", "9"]).await?;
    sqlx::query("DELETE FROM playlist_track_projection WHERE sc_track_id = '9'")
        .execute(&pool)
        .await?;
    let journal = PlaylistJournal::new(pool.clone());

    journal
        .append(
            PLAYLIST,
            OWNER,
            body(serde_json::json!({ "add": "soundcloud:tracks:9" })),
            Uuid::now_v7(),
        )
        .await?;
    journal
        .append(
            PLAYLIST,
            OWNER,
            body(serde_json::json!({ "remove": "soundcloud:tracks:2" })),
            Uuid::now_v7(),
        )
        .await?;

    assert_eq!(projection(&pool).await?, vec!["1", "3", "9"]);
    let positions = sqlx::query_scalar::<_, i32>(
        "SELECT position FROM playlist_track_projection
         WHERE playlist_urn = $1 ORDER BY position",
    )
    .bind(PLAYLIST)
    .fetch_all(&pool)
    .await?;
    assert_eq!(positions, vec![0, 2, 3]);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_removal_only_drops_the_named_track(pool: PgPool) -> anyhow::Result<()> {
    test_schema::install(&pool).await?;
    test_schema::seed_playlist(&pool, PLAYLIST, OWNER, &["1", "2", "3"]).await?;
    let journal = PlaylistJournal::new(pool.clone());

    journal
        .append(
            PLAYLIST,
            OWNER,
            body(serde_json::json!({ "remove": "soundcloud:tracks:2" })),
            Uuid::now_v7(),
        )
        .await?;

    assert_eq!(projection(&pool).await?, vec!["1", "3"]);
    assert_eq!(
        journalled(&pool).await?,
        vec![(1, "remove".to_owned(), Some("2".to_owned()))]
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_move_is_journalled_with_anchors_rather_than_an_index(
    pool: PgPool,
) -> anyhow::Result<()> {
    test_schema::install(&pool).await?;
    test_schema::seed_playlist(&pool, PLAYLIST, OWNER, &["1", "2", "3", "4"]).await?;
    let journal = PlaylistJournal::new(pool.clone());

    let request = MembershipRequest {
        edit: TrackEdit::Move {
            track_id: "4".to_owned(),
            to_index: 1,
        },
        expected_projection_revision: None,
    };
    journal
        .append(PLAYLIST, OWNER, request, Uuid::now_v7())
        .await?;

    assert_eq!(projection(&pool).await?, vec!["1", "4", "2", "3"]);
    let anchors = sqlx::query_as::<_, (Option<String>, Option<String>, Option<String>)>(
        "SELECT left_anchor_track_id, right_anchor_track_id, boundary
         FROM playlist_membership_operations WHERE playlist_urn = $1",
    )
    .bind(PLAYLIST)
    .fetch_one(&pool)
    .await?;
    assert_eq!(anchors, (Some("1".to_owned()), Some("2".to_owned()), None));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_order_request_never_removes_a_track_the_client_did_not_list(
    pool: PgPool,
) -> anyhow::Result<()> {
    test_schema::install(&pool).await?;
    test_schema::seed_playlist(&pool, PLAYLIST, OWNER, &["1", "2", "3"]).await?;
    let journal = PlaylistJournal::new(pool.clone());

    journal
        .append(
            PLAYLIST,
            OWNER,
            body(serde_json::json!({
                "order": ["soundcloud:tracks:3", "soundcloud:tracks:1"]
            })),
            Uuid::now_v7(),
        )
        .await?;

    assert_eq!(projection(&pool).await?, vec!["3", "2", "1"]);
    let kinds: Vec<String> = journalled(&pool)
        .await?
        .into_iter()
        .map(|(_, kind, _)| kind)
        .collect();
    assert_eq!(kinds, vec!["reorder".to_owned()]);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_order_request_adds_tracks_the_client_introduced(pool: PgPool) -> anyhow::Result<()> {
    test_schema::install(&pool).await?;
    test_schema::seed_playlist(&pool, PLAYLIST, OWNER, &["1", "2", "9"]).await?;
    sqlx::query("DELETE FROM playlist_track_projection WHERE sc_track_id = '9'")
        .execute(&pool)
        .await?;
    let journal = PlaylistJournal::new(pool.clone());

    journal
        .append(
            PLAYLIST,
            OWNER,
            body(serde_json::json!({
                "order": [
                    "soundcloud:tracks:1",
                    "soundcloud:tracks:9",
                    "soundcloud:tracks:2"
                ]
            })),
            Uuid::now_v7(),
        )
        .await?;

    assert_eq!(projection(&pool).await?, vec!["1", "9", "2"]);
    let kinds: Vec<String> = journalled(&pool)
        .await?
        .into_iter()
        .map(|(_, kind, _)| kind)
        .collect();
    assert_eq!(kinds, vec!["add".to_owned()]);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_replacing_write_without_a_client_revision_never_removes(
    pool: PgPool,
) -> anyhow::Result<()> {
    test_schema::install(&pool).await?;
    test_schema::seed_playlist(&pool, PLAYLIST, OWNER, &["1", "2", "3"]).await?;
    let journal = PlaylistJournal::new(pool.clone());

    journal
        .append(
            PLAYLIST,
            OWNER,
            MembershipRequest {
                edit: TrackEdit::Replace {
                    track_ids: vec!["3".to_owned(), "1".to_owned()],
                },
                expected_projection_revision: None,
            },
            Uuid::now_v7(),
        )
        .await?;

    assert_eq!(projection(&pool).await?, vec!["3", "2", "1"]);
    let kinds: Vec<String> = journalled(&pool)
        .await?
        .into_iter()
        .map(|(_, kind, _)| kind)
        .collect();
    assert_eq!(kinds, vec!["reorder".to_owned()]);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_committed_write_replays_instead_of_failing_its_revision_precondition(
    pool: PgPool,
) -> anyhow::Result<()> {
    test_schema::install(&pool).await?;
    test_schema::seed_playlist(&pool, PLAYLIST, OWNER, &["1", "2", "3"]).await?;
    let journal = PlaylistJournal::new(pool.clone());
    let key = Uuid::now_v7();
    let request = || MembershipRequest {
        edit: TrackEdit::Replace {
            track_ids: vec!["3".to_owned(), "1".to_owned()],
        },
        expected_projection_revision: Some(0),
    };

    journal.append(PLAYLIST, OWNER, request(), key).await?;
    let replay = journal.append(PLAYLIST, OWNER, request(), key).await?;

    assert_eq!(replay.appended, 0);
    assert_eq!(projection(&pool).await?, vec!["3", "1"]);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_replacing_write_with_a_matching_revision_removes_explicitly(
    pool: PgPool,
) -> anyhow::Result<()> {
    test_schema::install(&pool).await?;
    test_schema::seed_playlist(&pool, PLAYLIST, OWNER, &["1", "2", "3"]).await?;
    let journal = PlaylistJournal::new(pool.clone());

    journal
        .append(
            PLAYLIST,
            OWNER,
            MembershipRequest {
                edit: TrackEdit::Replace {
                    track_ids: vec!["3".to_owned(), "1".to_owned()],
                },
                expected_projection_revision: Some(0),
            },
            Uuid::now_v7(),
        )
        .await?;

    assert_eq!(projection(&pool).await?, vec!["3", "1"]);
    let kinds: Vec<String> = journalled(&pool)
        .await?
        .into_iter()
        .map(|(_, kind, _)| kind)
        .collect();
    assert_eq!(kinds, vec!["remove".to_owned(), "reorder".to_owned()]);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_stale_client_revision_is_rejected(pool: PgPool) -> anyhow::Result<()> {
    test_schema::install(&pool).await?;
    test_schema::seed_playlist(&pool, PLAYLIST, OWNER, &["1", "2"]).await?;
    let journal = PlaylistJournal::new(pool.clone());

    let error = journal
        .append(
            PLAYLIST,
            OWNER,
            body(serde_json::json!({
                "remove": "soundcloud:tracks:1",
                "expectedProjectionRevision": 7
            })),
            Uuid::now_v7(),
        )
        .await
        .expect_err("stale revision");

    assert!(error.to_string().contains(REVISION_CONFLICT));
    assert_eq!(projection(&pool).await?, vec!["1", "2"]);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_playlist_without_a_baseline_refuses_and_becomes_due(pool: PgPool) -> anyhow::Result<()> {
    test_schema::install(&pool).await?;
    test_schema::seed_playlist(&pool, PLAYLIST, OWNER, &["1", "2"]).await?;
    sqlx::query(
        "UPDATE playlist_membership_state
         SET baseline_generation = 0,
             baseline_observation_id = NULL,
             sync_status = 'unhydrated',
             next_reconcile_at = clock_timestamp() + interval '1 hour'
         WHERE playlist_urn = $1",
    )
    .bind(PLAYLIST)
    .execute(&pool)
    .await?;
    let journal = PlaylistJournal::new(pool.clone());

    let error = journal
        .append(
            PLAYLIST,
            OWNER,
            body(serde_json::json!({ "remove": "soundcloud:tracks:1" })),
            Uuid::now_v7(),
        )
        .await
        .expect_err("missing baseline");

    assert!(error.to_string().contains(AWAITING_BASELINE));
    let due: bool = sqlx::query_scalar(
        "SELECT next_reconcile_at <= clock_timestamp()
         FROM playlist_membership_state WHERE playlist_urn = $1",
    )
    .bind(PLAYLIST)
    .fetch_one(&pool)
    .await?;
    assert!(due);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_playlist_with_unresolved_legacy_intent_refuses(pool: PgPool) -> anyhow::Result<()> {
    test_schema::install(&pool).await?;
    test_schema::seed_playlist(&pool, PLAYLIST, OWNER, &["1", "2"]).await?;
    sqlx::query(
        "INSERT INTO playlist_legacy_membership_intents (
             archive_id, source, playlist_urn,
             legacy_desired_revision, legacy_synced_revision
         ) VALUES ($1, 'revision', $2, 2, 1)",
    )
    .bind(Uuid::now_v7())
    .bind(PLAYLIST)
    .execute(&pool)
    .await?;
    let journal = PlaylistJournal::new(pool.clone());

    let error = journal
        .append(
            PLAYLIST,
            OWNER,
            body(serde_json::json!({ "remove": "soundcloud:tracks:1" })),
            Uuid::now_v7(),
        )
        .await
        .expect_err("legacy intent");

    assert!(error.to_string().contains(LEGACY_RECONCILIATION_PENDING));
    assert_eq!(projection(&pool).await?, vec!["1", "2"]);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_track_outside_the_catalog_cannot_be_added(pool: PgPool) -> anyhow::Result<()> {
    test_schema::install(&pool).await?;
    test_schema::seed_playlist(&pool, PLAYLIST, OWNER, &["1", "2"]).await?;
    let journal = PlaylistJournal::new(pool.clone());

    let error = journal
        .append(
            PLAYLIST,
            OWNER,
            body(serde_json::json!({ "add": "soundcloud:tracks:404" })),
            Uuid::now_v7(),
        )
        .await
        .expect_err("unknown track");

    assert!(error.to_string().contains(UNKNOWN_TRACK));
    assert_eq!(projection(&pool).await?, vec!["1", "2"]);
    assert!(journalled(&pool).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_move_of_an_absent_track_changes_nothing(pool: PgPool) -> anyhow::Result<()> {
    test_schema::install(&pool).await?;
    test_schema::seed_playlist(&pool, PLAYLIST, OWNER, &["1", "2"]).await?;
    let journal = PlaylistJournal::new(pool.clone());

    let request = serde_json::from_value::<EditBody>(serde_json::json!({
        "move": { "track": "soundcloud:tracks:404", "to": 0 }
    }))?
    .into_request()?;
    let outcome = journal
        .append(PLAYLIST, OWNER, request, Uuid::now_v7())
        .await?;

    assert_eq!(outcome.appended, 0);
    assert_eq!(state(&pool).await?.3, "clean".to_owned());
    Ok(())
}

#[test]
fn exactly_one_membership_field_is_accepted() {
    let both = serde_json::from_value::<EditBody>(serde_json::json!({
        "add": "soundcloud:tracks:1",
        "remove": "soundcloud:tracks:2"
    }))
    .expect("edit body")
    .into_request();

    assert!(both.is_err());
}

#[test]
fn a_move_body_still_parses_the_legacy_index_shape() {
    let request = serde_json::from_value::<EditBody>(serde_json::json!({
        "move": { "track": "soundcloud:tracks:1", "to": 3 }
    }))
    .expect("edit body");

    let moved = request.move_op.clone().expect("move body");
    assert_eq!(moved.track, "soundcloud:tracks:1");
    assert_eq!(moved.to, 3);
    assert!(matches!(
        request.into_request().expect("request").edit,
        TrackEdit::Move { to_index: 3, .. }
    ));
}

#[test]
fn a_move_body_type_is_reachable_from_the_module_surface() {
    let moved = MoveBody {
        track: "soundcloud:tracks:1".to_owned(),
        to: 0,
    };

    assert_eq!(moved.to, 0);
}

#[sqlx::test(migrations = false)]
async fn abandoning_the_last_legacy_intent_unblocks_the_playlist(
    pool: PgPool,
) -> anyhow::Result<()> {
    test_schema::install(&pool).await?;
    test_schema::seed_playlist(&pool, PLAYLIST, OWNER, &["1", "2"]).await?;
    let archive_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO playlist_legacy_membership_intents (
             archive_id, source, playlist_urn,
             legacy_desired_revision, legacy_synced_revision, classification
         ) VALUES ($1, 'revision', $2, 2, 1, 'local_superset')",
    )
    .bind(archive_id)
    .bind(PLAYLIST)
    .execute(&pool)
    .await?;
    let journal = PlaylistJournal::new(pool.clone());
    let blocked = journal
        .append(
            PLAYLIST,
            OWNER,
            body(serde_json::json!({ "remove": "soundcloud:tracks:1" })),
            Uuid::now_v7(),
        )
        .await
        .expect_err("legacy intent blocks the write");
    assert!(blocked.to_string().contains(LEGACY_RECONCILIATION_PENDING));

    let playlist_urn =
        sqlx::query_file_scalar!("queries/admin/playlists/legacy_abandon.sql", archive_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(playlist_urn, PLAYLIST);
    let unblocked = sqlx::query_file!("queries/admin/playlists/legacy_wake.sql", PLAYLIST)
        .execute(&pool)
        .await?
        .rows_affected();
    assert_eq!(unblocked, 1);

    journal
        .append(
            PLAYLIST,
            OWNER,
            body(serde_json::json!({ "remove": "soundcloud:tracks:1" })),
            Uuid::now_v7(),
        )
        .await?;

    assert_eq!(projection(&pool).await?, vec!["2"]);
    let prior: Option<String> = sqlx::query_scalar(
        "SELECT prior_classification FROM playlist_legacy_membership_intents WHERE archive_id = $1",
    )
    .bind(archive_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(prior.as_deref(), Some("local_superset"));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn membership_journal_rolls_back_with_the_callers_transaction(
    pool: PgPool,
) -> anyhow::Result<()> {
    test_schema::install(&pool).await?;
    test_schema::seed_playlist(&pool, PLAYLIST, OWNER, &["1", "2", "3"]).await?;
    let before = state(&pool).await?;
    let journal = PlaylistJournal::new(pool.clone());
    let mut transaction = pool.begin().await?;
    let outcome = journal
        .append_in(
            &mut transaction,
            PLAYLIST,
            OWNER,
            body(serde_json::json!({"remove": "soundcloud:tracks:2"})),
            Uuid::now_v7(),
        )
        .await?;
    assert_eq!(outcome.appended, 1);
    transaction.rollback().await?;
    assert_eq!(projection(&pool).await?, ["1", "2", "3"]);
    assert!(journalled(&pool).await?.is_empty());
    assert_eq!(state(&pool).await?, before);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn deleted_tracks_cannot_be_added_to_a_playlist(pool: PgPool) -> anyhow::Result<()> {
    test_schema::install(&pool).await?;
    test_schema::seed_playlist(&pool, PLAYLIST, OWNER, &["1", "2", "3"]).await?;
    sqlx::query("DELETE FROM playlist_track_projection WHERE sc_track_id = '3'")
        .execute(&pool)
        .await?;
    sqlx::query("UPDATE tracks SET deleted_at = now() WHERE sc_track_id = '3'")
        .execute(&pool)
        .await?;
    let journal = PlaylistJournal::new(pool.clone());
    let result = journal
        .append(
            PLAYLIST,
            OWNER,
            body(serde_json::json!({"add": "soundcloud:tracks:3"})),
            Uuid::now_v7(),
        )
        .await;
    assert!(result.is_err());
    assert_eq!(projection(&pool).await?, ["1", "2"]);
    assert!(journalled(&pool).await?.is_empty());
    Ok(())
}

async fn playlist_adds(pool: &PgPool) -> anyhow::Result<Vec<(String, String, f64, bool)>> {
    let rows = sqlx::query_as::<_, (String, String, f64, bool)>(
        "SELECT sc_user_id, sc_track_id, weight,
                created_at BETWEEN (now() AT TIME ZONE 'UTC') - interval '1 minute'
                               AND (now() AT TIME ZONE 'UTC') + interval '1 minute'
         FROM user_events WHERE event_type = 'playlist_add' ORDER BY sc_track_id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

async fn detach(pool: &PgPool, track_ids: &[&str]) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM playlist_track_projection WHERE sc_track_id = ANY($1)")
        .bind(track_ids)
        .execute(pool)
        .await?;
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_added_track_is_a_playlist_add_at_the_time_of_the_request_and_only_once(
    pool: PgPool,
) -> anyhow::Result<()> {
    test_schema::install(&pool).await?;
    test_schema::seed_playlist(&pool, PLAYLIST, OWNER, &["1", "2", "9"]).await?;
    detach(&pool, &["9"]).await?;
    let journal = PlaylistJournal::new(pool.clone());
    let key = Uuid::now_v7();
    for _ in 0..2 {
        journal
            .append(
                PLAYLIST,
                OWNER,
                body(serde_json::json!({ "add": "soundcloud:tracks:9" })),
                key,
            )
            .await?;
    }
    assert_eq!(
        playlist_adds(&pool).await?,
        [(OWNER.to_owned(), "9".to_owned(), 0.9, true)]
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn removing_moving_and_reordering_add_nothing(pool: PgPool) -> anyhow::Result<()> {
    test_schema::install(&pool).await?;
    test_schema::seed_playlist(&pool, PLAYLIST, OWNER, &["1", "2", "3"]).await?;
    let journal = PlaylistJournal::new(pool.clone());
    for request in [
        serde_json::json!({ "remove": "soundcloud:tracks:3" }),
        serde_json::json!({ "move": { "track": "soundcloud:tracks:1", "to": 1 } }),
        serde_json::json!({ "order": ["soundcloud:tracks:1", "soundcloud:tracks:2"] }),
    ] {
        journal
            .append(PLAYLIST, OWNER, body(request), Uuid::now_v7())
            .await?;
    }
    assert!(playlist_adds(&pool).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_order_request_records_only_the_tracks_it_introduced(
    pool: PgPool,
) -> anyhow::Result<()> {
    test_schema::install(&pool).await?;
    test_schema::seed_playlist(&pool, PLAYLIST, OWNER, &["1", "2", "8", "9"]).await?;
    detach(&pool, &["8", "9"]).await?;
    sqlx::query("INSERT INTO disliked_tracks (sc_user_id, sc_track_id) VALUES ($1, '8')")
        .bind(format!("soundcloud:users:{OWNER}"))
        .execute(&pool)
        .await?;
    let journal = PlaylistJournal::new(pool.clone());
    journal
        .append(
            PLAYLIST,
            OWNER,
            body(serde_json::json!({
                "order": [
                    "soundcloud:tracks:9",
                    "soundcloud:tracks:1",
                    "soundcloud:tracks:8",
                    "soundcloud:tracks:2"
                ]
            })),
            Uuid::now_v7(),
        )
        .await?;
    assert_eq!(
        playlist_adds(&pool).await?,
        [(OWNER.to_owned(), "9".to_owned(), 0.9, true)]
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_rolled_back_addition_leaves_no_playlist_add(pool: PgPool) -> anyhow::Result<()> {
    test_schema::install(&pool).await?;
    test_schema::seed_playlist(&pool, PLAYLIST, OWNER, &["1", "9"]).await?;
    detach(&pool, &["9"]).await?;
    let journal = PlaylistJournal::new(pool.clone());
    let mut transaction = pool.begin().await?;
    journal
        .append_in(
            &mut transaction,
            PLAYLIST,
            OWNER,
            body(serde_json::json!({ "add": "soundcloud:tracks:9" })),
            Uuid::now_v7(),
        )
        .await?;
    transaction.rollback().await?;
    assert!(playlist_adds(&pool).await?.is_empty());
    Ok(())
}

#[test]
fn only_tracks_new_to_the_playlist_count_as_added() {
    let before = ["1".to_owned(), "2".to_owned()];
    let after = [
        "2".to_owned(),
        "3".to_owned(),
        "1".to_owned(),
        "3".to_owned(),
    ];
    assert_eq!(super::journal::newly_added(&before, &after), ["3"]);
    assert!(super::journal::newly_added(&after, &before).is_empty());
}
