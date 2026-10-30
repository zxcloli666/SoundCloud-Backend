use chrono::{DateTime, Utc};
use serde_json::json;
use uuid::Uuid;
use write_through as wt;
use wt::{ConnectionDropOrphan, SyncQueuePurge, SyncQueueRevive};

use crate::harness::Pair;

const QUEUE: &str = "INSERT INTO public.sync_queue (id, user_id, action_type, target_urn, payload, retry_count, dead, failed_at, next_run_at, lease_id, lease_generation, locked_at) VALUES
    ('10000000-0000-4000-8000-000000000001', '11', 'track_update', 'soundcloud:tracks:1', '{\"track\": {\"title\": \"a\"}}', 5, true, '2026-10-30 09:00:00+00', 'infinity', NULL, NULL, NULL),
    ('10000000-0000-4000-8000-000000000002', '11', 'like_track', 'soundcloud:tracks:2', NULL, 5, true, '2026-10-30 09:00:00+00', 'infinity', NULL, NULL, NULL),
    ('10000000-0000-4000-8000-000000000003', '11', 'playlist_membership', 'soundcloud:playlists:77', '{}', 5, true, '2026-10-30 09:00:00+00', 'infinity', NULL, NULL, NULL),
    ('10000000-0000-4000-8000-000000000004', '11', 'playlist_membership', 'soundcloud:playlists:78', '{}', 5, true, '2026-10-30 09:00:00+00', 'infinity', NULL, NULL, NULL),
    ('10000000-0000-4000-8000-000000000005', '11', 'track_update', 'soundcloud:tracks:5', '{\"track\": {\"title\": \"b\"}}', 5, true, '2026-09-01 09:00:00+00', 'infinity', NULL, NULL, NULL),
    ('10000000-0000-4000-8000-000000000006', '12', 'follow_user', 'soundcloud:users:6', NULL, 7, false, NULL, '2026-10-30 09:00:00+00', NULL, NULL, NULL),
    ('10000000-0000-4000-8000-000000000007', '12', 'unfollow_user', 'soundcloud:users:7', NULL, 7, false, NULL, '2026-10-30 09:00:00+00', 'aaaaaaaa-0000-4000-8000-000000000007', 1, '2026-10-30 09:59:00+00'),
    ('10000000-0000-4000-8000-000000000008', '12', 'like_playlist', 'soundcloud:playlists:8', NULL, 1, false, NULL, '2026-10-30 09:00:00+00', NULL, NULL, NULL)";

const CONNECTIONS: &str = "INSERT INTO public.soundcloud_connections (id, soundcloud_user_id, username, access_token, refresh_token, expires_at, scope) VALUES
    ('20000000-0000-4000-8000-000000000001', '31', 'left', 'a1', 'r1', '2026-11-01 00:00:00+00', ''),
    ('20000000-0000-4000-8000-000000000002', '32', 'kept', 'a2', 'r2', '2026-11-01 00:00:00+00', '');
    INSERT INTO public.sessions (id, soundcloud_connection_id) VALUES
    ('30000000-0000-4000-8000-000000000002', '20000000-0000-4000-8000-000000000002')";

fn at(stamp: &str) -> DateTime<Utc> {
    stamp.parse().unwrap()
}

async fn ids(pool: &sqlx::PgPool, sql: &str) -> Vec<String> {
    sqlx::query_scalar(sql).fetch_all(pool).await.unwrap()
}

async fn receipts(pool: &sqlx::PgPool, tbl: &str) -> Vec<String> {
    ids(
        pool,
        &format!("SELECT pk ->> 'id' FROM public.repl_purges WHERE tbl = '{tbl}' ORDER BY 1"),
    )
    .await
}

#[tokio::test]
async fn revive_runs_on_core_only_and_drops_rereads_with_a_receipt() {
    let pair = Pair::new(QUEUE).await;
    let revive = SyncQueueRevive {
        user_ids: vec!["11".to_owned(), "soundcloud:users:11".to_owned()],
        target_urn: None,
        dropped_targets: vec!["soundcloud:playlists:77".to_owned()],
        at: at("2026-10-30T10:00:00Z"),
    };
    let run = pair.run(&revive, &[]).await;
    assert_eq!(run.core, json!(2));
    assert_eq!(run.assert_same(), json!(null));
    assert_eq!(
        ids(
            &pair.core,
            "SELECT id::text FROM public.sync_queue WHERE user_id = '11' AND NOT dead ORDER BY 1"
        )
        .await,
        vec!["10000000-0000-4000-8000-000000000001"]
    );
    let next: DateTime<Utc> = sqlx::query_scalar("SELECT next_run_at FROM public.sync_queue WHERE id = '10000000-0000-4000-8000-000000000001'")
        .fetch_one(&pair.core)
        .await
        .unwrap();
    assert_eq!(next, at("2026-10-30T10:00:00Z"));
    assert_eq!(
        receipts(&pair.core, "sync_queue").await,
        vec!["10000000-0000-4000-8000-000000000003"]
    );
    assert_eq!(
        ids(
            &pair.api,
            "SELECT id::text FROM public.sync_queue WHERE dead ORDER BY 1"
        )
        .await
        .len(),
        5,
        "the api copy changes only when core's rows replicate back"
    );
    pair.finish().await;
}

#[tokio::test]
async fn admin_purge_keeps_leased_and_young_rows() {
    let pair = Pair::new(QUEUE).await;
    let purge = SyncQueuePurge {
        min_retries: 5,
        at: at("2026-10-30T10:00:00Z"),
    };
    let run = pair.run(&purge, &[]).await;
    assert_eq!(run.core, json!(6));
    run.assert_same();
    assert_eq!(
        ids(
            &pair.core,
            "SELECT id::text FROM public.sync_queue ORDER BY 1"
        )
        .await,
        vec![
            "10000000-0000-4000-8000-000000000007",
            "10000000-0000-4000-8000-000000000008"
        ]
    );
    assert_eq!(receipts(&pair.core, "sync_queue").await.len(), 6);
    assert_eq!(receipts(&pair.api, "sync_queue").await.len(), 0);
    pair.finish().await;
}

#[tokio::test]
async fn an_orphan_connection_is_purged_on_core_and_a_used_one_stays() {
    let pair = Pair::new(CONNECTIONS).await;
    let orphan = ConnectionDropOrphan {
        connection_id: "20000000-0000-4000-8000-000000000001"
            .parse::<Uuid>()
            .unwrap(),
    };
    let run = pair.run(&orphan, &[]).await;
    assert_eq!(run.core, json!(true));
    run.assert_same();
    let used = ConnectionDropOrphan {
        connection_id: "20000000-0000-4000-8000-000000000002"
            .parse::<Uuid>()
            .unwrap(),
    };
    let run = pair.run(&used, &[]).await;
    assert_eq!(run.core, json!(false));
    run.assert_same();
    assert_eq!(
        ids(
            &pair.core,
            "SELECT id::text FROM public.soundcloud_connections ORDER BY 1"
        )
        .await,
        vec!["20000000-0000-4000-8000-000000000002"]
    );
    assert_eq!(
        receipts(&pair.core, "soundcloud_connections").await,
        vec!["20000000-0000-4000-8000-000000000001"]
    );
    assert_eq!(
        ids(
            &pair.api,
            "SELECT id::text FROM public.soundcloud_connections ORDER BY 1"
        )
        .await
        .len(),
        2
    );
    pair.finish().await;
}
