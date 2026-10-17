use std::sync::Arc;

use sqlx::PgPool;
use uuid::Uuid;

use super::*;

fn service(pg: &PgPool) -> anyhow::Result<Arc<SyncQueueService>> {
    let redis = deadpool_redis::Config::from_url("redis://127.0.0.1:1")
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    Ok(SyncQueueService::new(pg.clone(), redis))
}

async fn oauth_app(pg: &PgPool, active: bool) -> anyhow::Result<Uuid> {
    let app = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO oauth_apps (id, name, client_id, client_secret, redirect_uri, active)
         VALUES ($1, 'test', $2, 'secret', 'https://app.test/', $3)",
    )
    .bind(app)
    .bind(app.to_string())
    .bind(active)
    .execute(pg)
    .await?;
    Ok(app)
}

async fn connect(pg: &PgPool, user: &str, app: Uuid, minutes_ago: i32) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO soundcloud_connections (
             id, soundcloud_user_id, oauth_app_id, access_token, refresh_token, expires_at,
             scope, updated_at
         ) VALUES ($1, $2, $3, 'access', 'refresh', now() + interval '1 hour', '',
                   now() - $4::int * interval '1 minute')",
    )
    .bind(Uuid::now_v7())
    .bind(user)
    .bind(app)
    .bind(minutes_ago)
    .execute(pg)
    .await?;
    Ok(())
}

async fn pause(
    pg: &PgPool,
    app: Uuid,
    action_type: &str,
    reason: &str,
    minutes_left: i32,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO sync_queue_action_pauses (oauth_app_id, action_type, paused_until, strikes, reason)
         VALUES ($1, $2, now() + $3::int * interval '1 minute', 1, $4)",
    )
    .bind(app)
    .bind(action_type)
    .bind(minutes_left)
    .bind(reason)
    .execute(pg)
    .await?;
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn sync_status_is_local_and_scoped_to_the_session_account(pg: PgPool) -> anyhow::Result<()> {
    let service = service(&pg)?;
    service
        .enqueue(
            "42",
            "track_update",
            "1",
            Some(&serde_json::json!({"track": {"title": "name"}})),
        )
        .await?;
    service
        .enqueue("soundcloud:users:42", "track_delete", "2", None)
        .await?;
    service.enqueue("99", "track_delete", "3", None).await?;
    sqlx::query("UPDATE sync_queue SET retry_count = 2 WHERE user_id = '42' AND action_type = 'track_delete'")
        .execute(&pg).await?;
    let status = service.status_for_user("soundcloud:users:42").await?;
    assert_eq!(
        serde_json::to_value(&status)?,
        serde_json::json!({
            "pendingCount": 1,
            "failedCount": 1,
            "delayedCount": 0,
            "retryInSec": null,
            "deadCount": 0,
            "needsRelogin": false,
            "playlistCreates": [],
            "playlistCreatesTruncated": false
        })
    );
    assert_eq!(service.status_for_user("42").await?, status);
    assert_eq!(service.status_for_user("99").await?.pending_count, 1);
    assert_eq!(service.status_for_user("100").await?.pending_count, 0);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn postponed_writes_report_when_soundcloud_will_be_retried(pg: PgPool) -> anyhow::Result<()> {
    let service = service(&pg)?;
    service.enqueue("7", "like_track", "1", None).await?;
    service.enqueue("7", "like_track", "2", None).await?;
    service.enqueue("7", "like_track", "3", None).await?;
    sqlx::query(
        "UPDATE sync_queue SET last_error = 'banned', next_run_at = now() + interval '20 minutes'
         WHERE user_id = '7' AND target_urn LIKE '%:1'",
    )
    .execute(&pg)
    .await?;
    sqlx::query(
        "UPDATE sync_queue SET retry_count = 1, last_error = 'boom', next_run_at = now() + interval '90 seconds'
         WHERE user_id = '7' AND target_urn LIKE '%:2'",
    )
    .execute(&pg)
    .await?;

    let status = service.status_for_user("7").await?;

    assert_eq!(status.pending_count, 2);
    assert_eq!(status.failed_count, 1);
    assert_eq!(status.delayed_count, 1);
    assert!(matches!(status.retry_in_sec, Some(1190..=1200)));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn writes_held_by_the_app_pause_are_delayed_until_the_pause_ends(
    pg: PgPool,
) -> anyhow::Result<()> {
    let service = service(&pg)?;
    let app = oauth_app(&pg, true).await?;
    connect(&pg, "7", app, 0).await?;
    pause(&pg, app, "like_track", "rate_limited", 40).await?;
    service.enqueue("7", "like_track", "1", None).await?;
    service.enqueue("7", "like_track", "2", None).await?;
    sqlx::query("UPDATE sync_queue SET retry_count = 2, last_error = 'HTTP 429' WHERE target_urn LIKE '%:2'")
        .execute(&pg)
        .await?;

    let status = service.status_for_user("7").await?;

    assert_eq!(status.pending_count, 2);
    assert_eq!(status.delayed_count, 2);
    assert_eq!(status.failed_count, 0);
    assert!(matches!(status.retry_in_sec, Some(2390..=2400)));
    assert!(!status.needs_relogin);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn dead_playlist_edits_count_as_dead_dead_likes_as_failing_dead_comments_nowhere(
    pg: PgPool,
) -> anyhow::Result<()> {
    let service = service(&pg)?;
    sqlx::query(
        "INSERT INTO sync_queue (user_id, action_type, target_urn, dead, failed_at, retry_count) VALUES
             ('8', 'playlist_update', 'soundcloud:playlists:1', true, now(), 5),
             ('8', 'like_track', 'soundcloud:tracks:1', true, now(), 5),
             ('8', 'comment', 'soundcloud:tracks:1', true, now(), 5)",
    )
    .execute(&pg)
    .await?;

    let status = service.status_for_user("8").await?;

    assert_eq!(status.pending_count, 0);
    assert_eq!(status.failed_count, 2);
    assert_eq!(status.dead_count, 1);
    assert_eq!(status.delayed_count, 0);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn edits_that_died_over_a_month_ago_count_neither_as_dead_nor_as_failing(
    pg: PgPool,
) -> anyhow::Result<()> {
    let service = service(&pg)?;
    sqlx::query(
        "INSERT INTO sync_queue (user_id, action_type, target_urn, dead, failed_at, retry_count) VALUES
             ('9', 'playlist_update', 'soundcloud:playlists:1', true, now() - interval '40 days', 5),
             ('9', 'playlist_create', 'new:1', true, now() - interval '1 day', 5)",
    )
    .execute(&pg)
    .await?;

    let status = service.status_for_user("9").await?;

    assert_eq!(status.dead_count, 1);
    assert_eq!(status.failed_count, 1);
    assert_eq!(service.pending_and_failed("9").await?, (0, 1));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn needs_relogin_only_when_a_new_login_can_help(pg: PgPool) -> anyhow::Result<()> {
    let service = service(&pg)?;
    let retired = oauth_app(&pg, false).await?;
    let good = oauth_app(&pg, true).await?;
    let refused = oauth_app(&pg, true).await?;
    pause(&pg, refused, "like_track", "subscription_required", -60).await?;
    connect(&pg, "1", retired, 0).await?;
    connect(&pg, "2", refused, 0).await?;
    connect(&pg, "3", good, 0).await?;

    assert!(service.status_for_user("1").await?.needs_relogin);
    assert!(service.status_for_user("2").await?.needs_relogin);
    assert!(!service.status_for_user("3").await?.needs_relogin);

    pause(&pg, good, "playlist_create", "subscription_required", 60).await?;
    assert!(!service.status_for_user("2").await?.needs_relogin);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn the_writer_is_the_good_app_even_when_the_refused_one_was_refreshed_later(
    pg: PgPool,
) -> anyhow::Result<()> {
    let service = service(&pg)?;
    let good = oauth_app(&pg, true).await?;
    let refused = oauth_app(&pg, true).await?;
    connect(&pg, "9", good, 60).await?;
    connect(&pg, "9", refused, 0).await?;
    pause(&pg, refused, "playlist_create", "subscription_required", 60).await?;
    pause(&pg, refused, "like_track", "rate_limited", 90).await?;
    pause(&pg, good, "like_track", "rate_limited", 30).await?;
    service.enqueue("9", "like_track", "1", None).await?;

    let status = service.status_for_user("9").await?;

    assert!(!status.needs_relogin);
    assert_eq!(status.delayed_count, 1);
    assert!(matches!(status.retry_in_sec, Some(1790..=1800)));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn playlist_creates_are_newest_first_with_state_and_truncate_at_100(
    pg: PgPool,
) -> anyhow::Result<()> {
    let service = service(&pg)?;
    sqlx::raw_sql(
        "INSERT INTO sync_queue (user_id, action_type, target_urn, payload, created_at)
         SELECT '10', 'playlist_create', 'new:' || n,
                jsonb_build_object('playlist', jsonb_build_object('title', 'P' || n)),
                now() - n * interval '1 minute'
         FROM generate_series(1, 101) AS n;
         INSERT INTO sync_queue (user_id, action_type, target_urn, payload, dead, failed_at)
         VALUES ('10', 'playlist_create', 'new:old', '{\"playlist\": {\"title\": \"Old\"}}',
                 true, now() - interval '40 days');
         UPDATE sync_queue SET last_error = 'limited', next_run_at = now() + interval '10 minutes'
         WHERE target_urn = 'new:2';
         UPDATE sync_queue SET dead = true, failed_at = now(), retry_count = 5
         WHERE target_urn = 'new:3';",
    )
    .execute(&pg)
    .await?;

    let status = service.status_for_user("10").await?;

    assert!(status.playlist_creates_truncated);
    assert_eq!(status.playlist_creates.len(), 100);
    assert_eq!(
        status.playlist_creates[..3],
        [
            PlaylistCreate {
                target_urn: "new:1".to_owned(),
                title: "P1".to_owned(),
                state: PlaylistCreateState::Queued,
            },
            PlaylistCreate {
                target_urn: "new:2".to_owned(),
                title: "P2".to_owned(),
                state: PlaylistCreateState::Delayed,
            },
            PlaylistCreate {
                target_urn: "new:3".to_owned(),
                title: "P3".to_owned(),
                state: PlaylistCreateState::Failed,
            },
        ]
    );
    assert_eq!(status.playlist_creates[99].target_urn, "new:100");
    assert_eq!(
        serde_json::to_value(&status.playlist_creates[2])?,
        serde_json::json!({"targetUrn": "new:3", "title": "P3", "state": "failed"})
    );
    Ok(())
}

async fn dead_rows(pg: &PgPool) -> anyhow::Result<Vec<(String, bool)>> {
    Ok(
        sqlx::query_as("SELECT action_type || ' ' || target_urn, dead FROM sync_queue ORDER BY 1")
            .fetch_all(pg)
            .await?,
    )
}

#[sqlx::test(migrations = "./migrations")]
async fn retry_revives_only_dead_rows_counted_as_dead_between_two_minutes_and_a_month(
    pg: PgPool,
) -> anyhow::Result<()> {
    let service = service(&pg)?;
    sqlx::query(
        "INSERT INTO sync_queue (
             user_id, action_type, target_urn, dead, failed_at, retry_count, last_error,
             next_run_at, remote_attempted_generation
         ) VALUES
             ('11', 'playlist_create', 'new:a', true, now() - interval '5 minutes', 5, 'HTTP 502', 'infinity', 1),
             ('11', 'playlist_update', 'soundcloud:playlists:5', true, now() - interval '5 minutes', 5, 'HTTP 500', 'infinity', 1),
             ('11', 'track_update', 'soundcloud:tracks:6', true, now() - interval '30 seconds', 5, 'HTTP 500', 'infinity', 1),
             ('11', 'like_track', 'soundcloud:tracks:7', true, now() - interval '5 minutes', 5, 'HTTP 404', 'infinity', 1),
             ('11', 'comment', 'soundcloud:tracks:8', true, now() - interval '5 minutes', 5, 'HTTP 502', 'infinity', 1),
             ('11', 'playlist_delete', 'soundcloud:playlists:9', true, now() - interval '40 days', 5, 'HTTP 500', 'infinity', 1)",
    )
    .execute(&pg)
    .await?;

    let one = service
        .revive("soundcloud:users:11", Some("new:a"), &[])
        .await?;
    let create: (bool, i32, Option<String>, bool, Option<i64>, bool) = sqlx::query_as(
        "SELECT dead, retry_count, last_error, failed_at IS NULL, remote_attempted_generation,
                next_run_at <= now()
         FROM sync_queue WHERE target_urn = 'new:a'",
    )
    .fetch_one(&pg)
    .await?;
    let rest = service.revive("11", None, &[]).await?;

    assert_eq!(one, 1);
    assert_eq!(create, (false, 0, None, true, Some(1), true));
    assert_eq!(rest, 1);
    assert_eq!(
        dead_rows(&pg).await?,
        [
            ("comment soundcloud:tracks:8".to_owned(), true),
            ("like_track soundcloud:tracks:7".to_owned(), true),
            ("playlist_create new:a".to_owned(), false),
            ("playlist_delete soundcloud:playlists:9".to_owned(), true),
            ("playlist_update soundcloud:playlists:5".to_owned(), false),
            ("track_update soundcloud:tracks:6".to_owned(), true),
        ]
    );
    assert_eq!(service.status_for_user("11").await?.dead_count, 1);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn retry_drops_only_the_stale_membership_rows_whose_fresh_read_was_queued(
    pg: PgPool,
) -> anyhow::Result<()> {
    let service = service(&pg)?;
    sqlx::query(
        "INSERT INTO sync_queue (user_id, action_type, target_urn, payload, dead, failed_at, retry_count)
         VALUES ('12', 'playlist_membership', 'soundcloud:playlists:77',
                 '{\"tracks\": [\"1\"], \"fingerprint\": \"aa\", \"reconcile_generation\": 3}',
                 true, now() - interval '5 minutes', 5),
                ('12', 'playlist_membership', 'soundcloud:playlists:78',
                 '{\"tracks\": [\"2\"], \"fingerprint\": \"bb\", \"reconcile_generation\": 4}',
                 true, now() - interval '5 minutes', 5)",
    )
    .execute(&pg)
    .await?;

    let memberships = service.dead_memberships("12", None).await?;
    let revived = service
        .revive("12", None, &["soundcloud:playlists:77".to_owned()])
        .await?;

    assert_eq!(
        memberships,
        ["soundcloud:playlists:77", "soundcloud:playlists:78"]
    );
    assert_eq!(revived, 1);
    assert_eq!(
        dead_rows(&pg).await?,
        [(
            "playlist_membership soundcloud:playlists:78".to_owned(),
            true
        )]
    );
    Ok(())
}

#[test]
fn cached_status_round_trips_and_rejects_the_old_format() {
    let counts = SyncCounts {
        pending_count: 3,
        failed_count: 1,
        delayed_count: 2,
        retry_in_sec: Some(600),
        dead_count: 1,
        needs_relogin: true,
        playlist_creates: vec![PlaylistCreate {
            target_urn: "new:1".to_owned(),
            title: "Mix".to_owned(),
            state: PlaylistCreateState::Delayed,
        }],
        playlist_creates_truncated: false,
    };
    let cached = serde_json::to_string(&counts).unwrap();
    assert_eq!(serde_json::from_str::<SyncCounts>(&cached).unwrap(), counts);
    assert!(serde_json::from_str::<SyncCounts>("12:3").is_err());
    assert!(
        serde_json::from_str::<SyncCounts>(
            r#"{"pendingCount":3,"failedCount":1,"delayedCount":2,"retryInSec":600}"#
        )
        .is_err()
    );
}
