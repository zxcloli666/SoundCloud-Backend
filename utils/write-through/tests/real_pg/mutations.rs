use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use uuid::Uuid;
use write_through as wt;
use wt::{
    CommentPost, MembershipOp, MembershipOperation, OwnedDelete, OwnedInsert, PlaylistDelete,
    PlaylistEdit, TrackDelete, TrackEdit,
};

use crate::harness::{Pair, Verdict};

const OWNER: &str = "204411307";
const PLAYLIST: &str = "soundcloud:playlists:1990011";
const HLC_1: i64 = 28_693_349_570_866_705;
const HLC_2: i64 = 28_693_349_670_866_705;
const FIXTURE: &str = "INSERT INTO public.tracks (
        id, sc_track_id, urn, title, title_normalized, duration_ms, sharing, uploader_sc_user_id,
        sc_observation, created_at, updated_at)
    VALUES
        ('7a000000-0000-4000-8000-000000000501', '501', 'soundcloud:tracks:501', 'Night Drive', 'night drive', 213000, 'public', '204411307', 517000, '2026-01-15 12:00:00+00', '2026-10-28 08:00:00+00'),
        ('7a000000-0000-4000-8000-000000000502', '502', 'soundcloud:tracks:502', 'Dawn', 'dawn', 180000, 'public', '204411307', 517000, '2026-01-15 12:00:00+00', '2026-10-28 08:00:00+00'),
        ('7a000000-0000-4000-8000-000000000503', '503', 'soundcloud:tracks:503', 'Dusk', 'dusk', 190000, 'public', '99', 517000, '2026-01-15 12:00:00+00', '2026-10-28 08:00:00+00'),
        ('7a000000-0000-4000-8000-000000000504', '504', 'soundcloud:tracks:504', 'Noon', 'noon', 200000, 'public', '99', 517000, '2026-01-15 12:00:00+00', '2026-10-28 08:00:00+00');
    INSERT INTO public.playlists (
        urn, sc_playlist_id, title, title_normalized, track_count, sharing, owner_sc_user_id,
        sc_observation, created_at, updated_at)
    VALUES ('soundcloud:playlists:1990011', '1990011', 'Drive Mix', 'drive mix', 2, 'public', '204411307',
        517002, '2026-02-01 10:00:00+00', '2026-10-28 08:00:00+00');
    INSERT INTO public.playlist_membership_state (playlist_urn, projection_track_count, sync_status, next_reconcile_at, updated_at)
    VALUES ('soundcloud:playlists:1990011', 2, 'clean', '2026-11-01 00:00:00+00', '2026-10-28 08:00:00+00');
    INSERT INTO public.playlist_track_projection (playlist_urn, position, sc_track_id)
    VALUES ('soundcloud:playlists:1990011', 0, '501'), ('soundcloud:playlists:1990011', 1, '502');
    INSERT INTO public.user_owned_tracks (user_id, sc_track_id, created_at)
    VALUES ('204411307', '501', '2026-01-15 12:00:00+00');
    INSERT INTO public.user_owned_playlists (user_id, playlist_urn, created_at)
    VALUES ('204411307', 'soundcloud:playlists:1990011', '2026-02-01 10:00:00+00')";
const DERIVED: &[&str] = &["lyrics_lookup_state"];

fn at(stamp: &str) -> DateTime<Utc> {
    stamp.parse().unwrap()
}

fn fresh() -> Uuid {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    Uuid::from_u128(
        (0x0b00_0000_0000_4000_8000_u128 << 48) | u128::from(NEXT.fetch_add(1, Ordering::Relaxed)),
    )
}

fn variants() -> Vec<String> {
    vec![OWNER.to_owned(), format!("soundcloud:users:{OWNER}")]
}

async fn rows(pool: &sqlx::PgPool, sql: &str) -> Value {
    sqlx::query_scalar(&format!(
        "SELECT coalesce(jsonb_agg(r), '[]') FROM ({sql}) AS r"
    ))
    .fetch_one(pool)
    .await
    .unwrap_or_else(|e| panic!("{sql}: {e}"))
}

fn verdict(reasserted: i64, purged: i64) -> Verdict {
    Verdict {
        status: Some("ok".to_owned()),
        reasserted,
        purged,
        dead: 0,
    }
}

fn track_edit(desired: Value, observation: i64, stamp: &str) -> TrackEdit {
    TrackEdit {
        sc_track_id: "501".to_owned(),
        uploader: OWNER.to_owned(),
        desired,
        observation,
        at: at(stamp),
    }
}

fn track_delete(stamp: &str) -> TrackDelete {
    TrackDelete {
        sc_track_id: "501".to_owned(),
        uploader: OWNER.to_owned(),
        observation: HLC_1,
        at: at(stamp),
    }
}

fn playlist_edit(desired: Value, stamp: &str) -> PlaylistEdit {
    PlaylistEdit {
        urn: PLAYLIST.to_owned(),
        owner: OWNER.to_owned(),
        desired,
        observation: HLC_1,
        at: at(stamp),
    }
}

fn playlist_delete(stamp: &str) -> PlaylistDelete {
    PlaylistDelete {
        urn: PLAYLIST.to_owned(),
        owner: OWNER.to_owned(),
        observation: HLC_2,
        at: at(stamp),
    }
}

fn add(key: Uuid, track: &str) -> MembershipOperation {
    MembershipOperation {
        operation_id: fresh(),
        idempotency_key: key,
        fingerprint: "ab".repeat(32),
        kind: "add".to_owned(),
        track_id: Some(track.to_owned()),
        left_anchor_track_id: None,
        right_anchor_track_id: None,
        boundary: Some("back".to_owned()),
        ordered_track_ids: None,
    }
}

fn membership(expected: Option<i64>, operation: MembershipOperation, stamp: &str) -> MembershipOp {
    MembershipOp {
        playlist_urn: PLAYLIST.to_owned(),
        actor: OWNER.to_owned(),
        variants: variants(),
        expected_projection_revision: expected,
        operations: vec![operation],
        at: at(stamp),
    }
}

async fn projection(pool: &sqlx::PgPool) -> Value {
    rows(
        pool,
        "SELECT position, sc_track_id FROM public.playlist_track_projection ORDER BY position",
    )
    .await
}

async fn journal(pool: &sqlx::PgPool) -> Value {
    rows(
        pool,
        "SELECT sequence, track_id, outcome, conflict_code FROM public.playlist_membership_operations ORDER BY sequence",
    )
    .await
}

#[tokio::test]
async fn a_track_edit_runs_once_and_its_replay_changes_nothing() {
    let pair = Pair::new(FIXTURE).await;
    let edit = track_edit(
        json!({"title": "Night Drive (Edit)", "title_normalized": "night drive (edit)"}),
        HLC_1,
        "2026-10-30T10:00:00Z",
    );
    let output = pair.run(&edit, DERIVED).await.assert_same();
    assert_eq!(output, json!("soundcloud:tracks:501"));
    assert!(pair.redeliver().await);
    assert_eq!(
        rows(&pair.core, "SELECT title, sc_desired, sc_write_confirmed FROM public.tracks WHERE sc_track_id = '501'").await,
        json!([{"title": "Night Drive (Edit)", "sc_desired": {"title": "Night Drive (Edit)", "title_normalized": "night drive (edit)"}, "sc_write_confirmed": false}])
    );
    pair.finish().await;
}

#[tokio::test]
async fn track_edits_from_two_nodes_merge_on_core_and_the_late_node_is_reasserted() {
    let pair = Pair::with_star(FIXTURE).await;
    let title = track_edit(
        json!({"title": "Night Drive (Edit)", "title_normalized": "night drive (edit)"}),
        HLC_1,
        "2026-10-30T10:00:00Z",
    );
    pair.run(&title, DERIVED).await.assert_same();
    let description = track_edit(
        json!({"description": "Late"}),
        HLC_2,
        "2026-10-30T10:00:01Z",
    );
    let (output, star) = pair.run_on_star(&description).await;
    assert_eq!(output, json!("soundcloud:tracks:501"));
    assert_eq!(star, verdict(1, 0));
    assert_eq!(
        rows(
            &pair.core,
            "SELECT title, description, sc_desired FROM public.tracks WHERE sc_track_id = '501'"
        )
        .await,
        json!([{
            "title": "Night Drive (Edit)",
            "description": "Late",
            "sc_desired": {"title": "Night Drive (Edit)", "title_normalized": "night drive (edit)", "description": "Late"}
        }])
    );
    pair.finish().await;
}

#[tokio::test]
async fn an_edit_of_a_track_another_node_deleted_is_refused_on_core_and_reverted() {
    let pair = Pair::with_star(FIXTURE).await;
    let output = pair
        .run(&track_delete("2026-10-30T10:00:00Z"), DERIVED)
        .await
        .assert_same();
    assert_eq!(output, json!("soundcloud:tracks:501"));
    assert!(pair.redeliver().await);
    let edit = track_edit(json!({"title": "Lost"}), HLC_2, "2026-10-30T10:00:01Z");
    let (output, star) = pair.run_on_star(&edit).await;
    assert_eq!(output, json!("soundcloud:tracks:501"));
    assert_eq!(star, verdict(1, 0));
    assert_eq!(
        rows(&pair.core, "SELECT title, sharing, sc_desired, deleted_at IS NOT NULL AS deleted FROM public.tracks WHERE sc_track_id = '501'").await,
        json!([{"title": "Night Drive", "sharing": "private", "sc_desired": {}, "deleted": true}])
    );
    pair.finish().await;
}

#[tokio::test]
async fn a_track_deleted_on_both_nodes_keeps_the_first_deletion() {
    let pair = Pair::with_star(FIXTURE).await;
    pair.run(&track_delete("2026-10-30T10:00:00Z"), DERIVED)
        .await
        .assert_same();
    let (output, star) = pair
        .run_on_star(&track_delete("2026-10-30T10:00:05Z"))
        .await;
    assert_eq!(output, json!("soundcloud:tracks:501"));
    assert_eq!(star.status.as_deref(), Some("ok"));
    assert_eq!(star.dead, 0);
    assert_eq!(
        rows(&pair.core, "SELECT deleted_at = '2026-10-30T10:00:00Z' AS first FROM public.tracks WHERE sc_track_id = '501'").await,
        json!([{"first": true}])
    );
    pair.finish().await;
}

#[tokio::test]
async fn a_playlist_edit_runs_once_and_merges_with_the_other_node() {
    let pair = Pair::with_star(FIXTURE).await;
    let title = playlist_edit(
        json!({"title": "Night Mix", "title_normalized": "night mix"}),
        "2026-10-30T10:00:00Z",
    );
    let output = pair.run(&title, &[]).await.assert_same();
    assert_eq!(output, json!(PLAYLIST));
    assert!(pair.redeliver().await);
    let (output, star) = pair
        .run_on_star(&playlist_edit(
            json!({"sharing": "private"}),
            "2026-10-30T10:00:01Z",
        ))
        .await;
    assert_eq!(output, json!(PLAYLIST));
    assert_eq!(star, verdict(1, 0));
    assert_eq!(
        rows(
            &pair.core,
            "SELECT title, sharing, sc_desired FROM public.playlists"
        )
        .await,
        json!([{
            "title": "Night Mix",
            "sharing": "private",
            "sc_desired": {"title": "Night Mix", "title_normalized": "night mix", "sharing": "private"}
        }])
    );
    pair.finish().await;
}

#[tokio::test]
async fn a_playlist_delete_retires_membership_once_and_a_late_edit_is_reverted() {
    let pair = Pair::with_star(FIXTURE).await;
    let output = pair
        .run(&playlist_delete("2026-10-30T10:00:00Z"), &[])
        .await
        .assert_same();
    assert_eq!(output, json!(PLAYLIST));
    assert!(pair.redeliver().await);
    let (output, star) = pair
        .run_on_star(&playlist_edit(
            json!({"title": "Lost"}),
            "2026-10-30T10:00:01Z",
        ))
        .await;
    assert_eq!(output, json!(PLAYLIST));
    assert_eq!(star, verdict(1, 0));
    let (output, star) = pair
        .run_on_star(&playlist_delete("2026-10-30T10:00:02Z"))
        .await;
    assert_eq!(output, json!(PLAYLIST));
    assert_eq!(star.status.as_deref(), Some("ok"));
    assert_eq!(
        rows(&pair.core, "SELECT p.title, p.deleted_at = '2026-10-30T10:00:00Z' AS first, s.reconcile_generation, s.next_reconcile_at IS NULL AS retired FROM public.playlists p JOIN public.playlist_membership_state s ON s.playlist_urn = p.urn").await,
        json!([{"title": "Drive Mix", "first": true, "reconcile_generation": 2, "retired": true}])
    );
    pair.finish().await;
}

#[tokio::test]
async fn an_owned_track_is_inserted_once_across_nodes_and_deleted_once() {
    let pair = Pair::with_star(FIXTURE).await;
    let insert = |stamp: &str| OwnedInsert {
        user_id: OWNER.to_owned(),
        sc_track_id: "502".to_owned(),
        variants: variants(),
        at: at(stamp),
    };
    assert_eq!(
        pair.run(&insert("2026-10-30T10:00:00Z"), &[])
            .await
            .assert_same(),
        json!(true)
    );
    assert!(pair.redeliver().await);
    let (output, star) = pair.run_on_star(&insert("2026-10-30T10:00:01Z")).await;
    assert_eq!(output, json!(true));
    assert_eq!(star, verdict(1, 0));
    let delete = OwnedDelete::Track {
        variants: variants(),
        sc_track_id: "502".to_owned(),
    };
    assert_eq!(pair.run(&delete, &[]).await.assert_same(), json!(1));
    let (output, star) = pair.run_on_star(&delete).await;
    assert_eq!(output, json!(1));
    assert_eq!(star, verdict(0, 0));
    assert_eq!(
        rows(
            &pair.core,
            "SELECT sc_track_id FROM public.user_owned_tracks ORDER BY 1"
        )
        .await,
        json!([{"sc_track_id": "501"}])
    );
    let playlist = OwnedDelete::Playlist {
        variants: variants(),
        playlist_urn: PLAYLIST.to_owned(),
    };
    assert_eq!(pair.run(&playlist, &[]).await.assert_same(), json!(1));
    assert!(pair.redeliver().await);
    pair.finish().await;
}

#[tokio::test]
async fn a_comment_is_stored_once_even_when_its_command_reaches_core_twice() {
    let pair = Pair::with_star(FIXTURE).await;
    let post = |stamp: &str| CommentPost {
        id: "5c000000-0000-4000-8000-000000000001".parse().unwrap(),
        sc_track_id: "501".to_owned(),
        user_urn: format!("soundcloud:users:{OWNER}"),
        body: "first".to_owned(),
        track_position_ms: Some(250),
        at: at(stamp),
    };
    assert_eq!(
        pair.run(&post("2026-10-30T10:00:00Z"), &[])
            .await
            .assert_same(),
        json!(true)
    );
    assert!(pair.redeliver().await);
    let (output, star) = pair.run_on_star(&post("2026-10-30T10:00:01Z")).await;
    assert_eq!(output, json!(true));
    assert_eq!(star, verdict(1, 0));
    assert_eq!(
        rows(
            &pair.core,
            "SELECT body, created_at = '2026-10-30T10:00:00Z' AS first FROM public.track_comments"
        )
        .await,
        json!([{"body": "first", "first": true}])
    );
    pair.finish().await;
}

#[tokio::test]
async fn a_membership_add_runs_once_and_its_replay_changes_nothing() {
    let pair = Pair::new(FIXTURE).await;
    let op = membership(Some(0), add(fresh(), "503"), "2026-10-30T10:00:00Z");
    let output = pair.run(&op, &[]).await.assert_same();
    assert_eq!(
        output,
        json!({"status": "applied", "code": null, "appended": 1, "projection_revision": 1, "added": ["503"]})
    );
    assert!(pair.redeliver().await);
    assert_eq!(
        projection(&pair.core).await,
        json!([
            {"position": 0, "sc_track_id": "501"},
            {"position": 1, "sc_track_id": "502"},
            {"position": 2, "sc_track_id": "503"}
        ])
    );
    let again = membership(
        Some(1),
        add(op.operations[0].idempotency_key, "503"),
        "2026-10-30T10:00:05Z",
    );
    let output = pair.run(&again, &[]).await.assert_same();
    assert_eq!(output["status"], json!("replayed"));
    assert_eq!(
        journal(&pair.core).await,
        json!([{"sequence": 1, "track_id": "503", "outcome": "pending", "conflict_code": null}])
    );
    pair.finish().await;
}

#[tokio::test]
async fn the_same_idempotency_key_from_two_nodes_is_applied_once() {
    let pair = Pair::with_star(FIXTURE).await;
    let key = fresh();
    pair.run(
        &membership(None, add(key, "503"), "2026-10-30T10:00:00Z"),
        &[],
    )
    .await
    .assert_same();
    let (output, star) = pair
        .run_on_star(&membership(None, add(key, "503"), "2026-10-30T10:00:01Z"))
        .await;
    assert_eq!(output["status"], json!("applied"));
    assert_eq!(star, verdict(2, 1));
    assert_eq!(
        journal(&pair.core).await,
        json!([{"sequence": 1, "track_id": "503", "outcome": "pending", "conflict_code": null}])
    );
    assert_eq!(
        projection(&pair.core).await.as_array().map(Vec::len),
        Some(3)
    );
    pair.finish().await;
}

#[tokio::test]
async fn a_stale_revision_from_the_other_node_is_journalled_as_a_conflict_and_reverted() {
    let pair = Pair::with_star(FIXTURE).await;
    pair.run(
        &membership(Some(0), add(fresh(), "503"), "2026-10-30T10:00:00Z"),
        &[],
    )
    .await
    .assert_same();
    let (output, star) = pair
        .run_on_star(&membership(
            Some(0),
            add(fresh(), "504"),
            "2026-10-30T10:00:01Z",
        ))
        .await;
    assert_eq!(output["status"], json!("applied"));
    assert_eq!(star, verdict(3, 0));
    assert_eq!(
        journal(&pair.core).await,
        json!([
            {"sequence": 1, "track_id": "503", "outcome": "pending", "conflict_code": null},
            {"sequence": 2, "track_id": "504", "outcome": "conflict", "conflict_code": "playlist_revision_conflict"}
        ])
    );
    assert_eq!(
        projection(&pair.core).await,
        json!([
            {"position": 0, "sc_track_id": "501"},
            {"position": 1, "sc_track_id": "502"},
            {"position": 2, "sc_track_id": "503"}
        ])
    );
    assert_eq!(
        rows(&pair.core, "SELECT last_operation_sequence, committed_operation_sequence, projection_revision, projection_track_count FROM public.playlist_membership_state").await,
        json!([{"last_operation_sequence": 2, "committed_operation_sequence": 0, "projection_revision": 1, "projection_track_count": 3}])
    );
    pair.finish().await;
}

#[tokio::test]
async fn adds_without_a_revision_from_two_nodes_land_in_arrival_order() {
    let pair = Pair::with_star(FIXTURE).await;
    pair.run(
        &membership(None, add(fresh(), "503"), "2026-10-30T10:00:00Z"),
        &[],
    )
    .await
    .assert_same();
    let (output, star) = pair
        .run_on_star(&membership(
            None,
            add(fresh(), "504"),
            "2026-10-30T10:00:01Z",
        ))
        .await;
    assert_eq!(output["added"], json!(["504"]));
    assert_eq!(star, verdict(3, 0));
    assert_eq!(
        projection(&pair.core).await,
        json!([
            {"position": 0, "sc_track_id": "501"},
            {"position": 1, "sc_track_id": "502"},
            {"position": 2, "sc_track_id": "503"},
            {"position": 3, "sc_track_id": "504"}
        ])
    );
    pair.finish().await;
}

#[tokio::test]
async fn a_membership_edit_by_someone_else_is_refused_on_both_nodes() {
    let pair = Pair::new(FIXTURE).await;
    let mut op = membership(None, add(fresh(), "503"), "2026-10-30T10:00:00Z");
    op.actor = "99".to_owned();
    op.variants = vec!["99".to_owned()];
    let output = pair.run(&op, &[]).await.assert_same();
    assert_eq!(output["status"], json!("refused"));
    assert_eq!(output["code"], json!("playlist_not_found"));
    assert_eq!(
        journal(&pair.core).await,
        json!([{"sequence": 1, "track_id": "503", "outcome": "conflict", "conflict_code": "playlist_not_found"}])
    );
    assert_eq!(
        projection(&pair.core).await.as_array().map(Vec::len),
        Some(2)
    );
    pair.finish().await;
}
