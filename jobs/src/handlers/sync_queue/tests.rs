use std::time::Duration;

use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::client::TokenRefreshClient;
use super::connection::ConnectionManager;
use super::model::ClaimedMutation;
use super::repository::SyncQueueRepository;

async fn install_schema(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE EXTENSION IF NOT EXISTS pgcrypto;
         CREATE TABLE sync_queue (
             id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
             user_id text NOT NULL,
             action_type varchar(32) NOT NULL,
             target_urn text NOT NULL,
             payload jsonb,
             locked_at timestamptz,
             retry_count integer NOT NULL DEFAULT 0,
             last_error text,
             next_run_at timestamptz NOT NULL DEFAULT now(),
             created_at timestamptz NOT NULL DEFAULT now(),
             dead boolean NOT NULL DEFAULT false,
             failed_at timestamptz
         );
         CREATE INDEX sync_queue_pickup_idx ON sync_queue (next_run_at, locked_at);
         CREATE UNIQUE INDEX sync_queue_target_uq
             ON sync_queue (user_id, action_type, target_urn);
         CREATE TABLE oauth_apps (
             id integer PRIMARY KEY,
             active boolean NOT NULL
         );
         INSERT INTO oauth_apps (id, active) VALUES (1, true), (2, false);
         CREATE TABLE soundcloud_connections (
             soundcloud_user_id text NOT NULL,
             oauth_app_id integer NOT NULL DEFAULT 1,
             last_refresh_error_kind text,
             retry_at timestamptz,
             expires_at timestamptz NOT NULL DEFAULT now() + interval '1 hour',
             updated_at timestamptz NOT NULL DEFAULT now()
         );
         INSERT INTO soundcloud_connections (soundcloud_user_id) VALUES ('1');
         CREATE TABLE sync_queue_action_pauses (
             oauth_app_id integer NOT NULL,
             action_type text NOT NULL,
             reason text NOT NULL
         );
         CREATE TABLE subscriptions (
             user_urn text PRIMARY KEY,
             exp_date bigint NOT NULL
         );
         CREATE TABLE user_likes_tracks (
             user_id text NOT NULL,
             sc_track_id text NOT NULL,
             progress boolean NOT NULL DEFAULT false,
             synced_at timestamptz,
             created_at timestamptz NOT NULL DEFAULT now()
         );
         CREATE TABLE user_likes_playlists (
             user_id text NOT NULL,
             playlist_urn text NOT NULL,
             progress boolean NOT NULL DEFAULT false,
             synced_at timestamptz,
             created_at timestamptz NOT NULL DEFAULT now()
         );
         CREATE TABLE user_followings (
             user_id text NOT NULL,
             target_user_urn text NOT NULL,
             progress boolean NOT NULL DEFAULT false,
             synced_at timestamptz,
             created_at timestamptz NOT NULL DEFAULT now()
         );",
    )
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../../api/migrations/0068_sync_queue_leases.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../../api/migrations/0070_sync_queue_remote_attempts.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../../api/migrations/0190_scd_versions_sync_queue.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(
        "CREATE FUNCTION scd_purge_intent(tbl text, k jsonb, seen bigint)
         RETURNS boolean
         LANGUAGE sql
         AS $f$
         WITH gone AS (
             DELETE FROM sync_queue WHERE id = (k ->> 'id')::uuid AND intent_v = seen RETURNING id
         )
         SELECT EXISTS (SELECT 1 FROM gone)
         $f$",
    )
    .execute(pool)
    .await?;
    Ok(())
}

fn repository(pool: PgPool) -> SyncQueueRepository {
    SyncQueueRepository::new(pool, Duration::from_secs(300))
}

#[sqlx::test(migrations = false)]
async fn comments_remain_distinct_while_state_actions_are_unique(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    sqlx::query(
        "INSERT INTO sync_queue (user_id, action_type, target_urn)
         VALUES ('1', 'comment', 'track'), ('1', 'comment', 'track')",
    )
    .execute(&pool)
    .await?;

    let duplicate = sqlx::query(
        "INSERT INTO sync_queue (user_id, action_type, target_urn)
         VALUES ('1', 'like_track', 'track'), ('1', 'like_track', 'track')",
    )
    .execute(&pool)
    .await;

    assert!(duplicate.is_err());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn later_intent_waits_behind_a_head_in_backoff(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    sqlx::query(
        "INSERT INTO sync_queue (
             user_id, action_type, target_urn, next_run_at, created_at
         ) VALUES
             ('1', 'like_track', 'track', now() + interval '1 hour', now()),
             ('1', 'unlike_track', 'track', now(), now() + interval '1 second')",
    )
    .execute(&pool)
    .await?;
    let repository = repository(pool.clone());

    assert!(repository.claim(2).await?.is_empty());

    sqlx::query("UPDATE sync_queue SET next_run_at = now() WHERE action_type = 'like_track'")
        .execute(&pool)
        .await?;
    let claimed = repository.claim(2).await?;
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].action_type, "like_track");
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn only_users_whose_login_can_still_be_refreshed_are_served(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    sqlx::query(
        "INSERT INTO sync_queue (user_id, action_type, target_urn, next_run_at) VALUES
             ('5', 'like_track', 'retired-app', now() - interval '3 days'),
             ('2', 'like_track', 'relogin', now() - interval '2 days'),
             ('3', 'like_track', 'cooling', now() - interval '1 day'),
             ('6', 'like_track', 'never-connected', now() - interval '1 day'),
             ('4', 'like_track', 'refreshable', now())",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO soundcloud_connections (
             soundcloud_user_id, oauth_app_id, last_refresh_error_kind, retry_at
         ) VALUES
             ('5', 2, NULL, NULL),
             ('2', 1, 'reauthorization_required', NULL),
             ('3', 1, NULL, now() + interval '5 minutes'),
             ('4', 1, NULL, NULL)",
    )
    .execute(&pool)
    .await?;
    let repository = repository(pool);

    let claimed = repository.claim(8).await?;

    let targets: Vec<&str> = claimed
        .iter()
        .map(|mutation| mutation.target_urn.as_str())
        .collect();
    assert_eq!(targets, vec!["refreshable"]);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn one_claim_carries_a_single_like_and_skips_actions_on_hold(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    sqlx::query(
        "INSERT INTO sync_queue (user_id, action_type, target_urn, next_run_at) VALUES
             ('1', 'like_track', 'first-like', now() - interval '3 minutes'),
             ('1', 'like_track', 'second-like', now() - interval '2 minutes'),
             ('1', 'follow_user', 'artist', now() - interval '1 minute'),
             ('1', 'playlist_update', 'playlist', now())",
    )
    .execute(&pool)
    .await?;
    let repository = repository(pool);

    let open = repository.claim_outside(8, &[], true).await?;
    let targets = |claimed: &[ClaimedMutation]| {
        claimed
            .iter()
            .map(|mutation| mutation.target_urn.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(targets(&open), ["first-like", "artist", "playlist"]);

    let held = repository
        .claim_outside(8, &["1:like_track".to_owned()], true)
        .await?;
    assert!(held.is_empty());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_like_on_hold_keeps_back_only_the_unlike_of_the_same_target(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    sqlx::query(
        "INSERT INTO sync_queue (user_id, action_type, target_urn, created_at) VALUES
             ('1', 'like_playlist', 'playlist', now() - interval '5 minutes'),
             ('1', 'playlist_membership', 'playlist', now() - interval '4 minutes'),
             ('1', 'like_track', 'track', now() - interval '3 minutes'),
             ('1', 'comment', 'track', now() - interval '2 minutes'),
             ('1', 'unlike_track', 'track', now() - interval '1 minute')",
    )
    .execute(&pool)
    .await?;
    let repository = repository(pool);

    let claimed = repository
        .claim_outside(
            8,
            &["1:like_track".to_owned(), "1:like_playlist".to_owned()],
            true,
        )
        .await?;

    let mut actions: Vec<&str> = claimed
        .iter()
        .map(|mutation| mutation.action_type.as_str())
        .collect();
    actions.sort_unstable();
    assert_eq!(actions, ["comment", "playlist_membership"]);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_edit_still_waits_for_the_earlier_edit_of_the_same_target(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    sqlx::query(
        "INSERT INTO sync_queue (user_id, action_type, target_urn, created_at, next_run_at) VALUES
             ('1', 'playlist_create', 'playlist', now() - interval '2 minutes', now() + interval '1 hour'),
             ('1', 'playlist_membership', 'playlist', now() - interval '1 minute', now()),
             ('1', 'like_playlist', 'playlist', now(), now())",
    )
    .execute(&pool)
    .await?;
    let repository = repository(pool);

    let claimed = repository.claim_outside(8, &[], true).await?;

    assert!(claimed.is_empty());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn terminal_intent_does_not_block_the_live_head(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    sqlx::query(
        "INSERT INTO sync_queue (
             user_id, action_type, target_urn, created_at, dead, failed_at
         ) VALUES
             ('1', 'comment', 'track', now(), true, now()),
             ('1', 'like_track', 'track', now() + interval '1 second', false, NULL)",
    )
    .execute(&pool)
    .await?;
    let repository = repository(pool);

    let claimed = repository.claim(2).await?;

    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].action_type, "like_track");
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn stale_remote_completion_cannot_delete_a_new_generation(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    sqlx::query(
        "INSERT INTO sync_queue (user_id, action_type, target_urn)
         VALUES ('1', 'like_track', 'track')",
    )
    .execute(&pool)
    .await?;
    let repository = repository(pool.clone());
    let mutation = repository.claim(1).await?.remove(0);
    sqlx::query(
        "UPDATE sync_queue
         SET generation = generation + 1,
             remote_attempted_generation = NULL,
             remote_completed_generation = NULL,
             remote_result = NULL,
             next_run_at = now()
         WHERE id = $1",
    )
    .bind(mutation.id)
    .execute(&pool)
    .await?;

    assert!(
        !repository
            .record_remote_success(&mutation, &json!({ "ok": true }))
            .await?
    );
    repository.release(&mutation).await?;
    let state: (i64, bool) =
        sqlx::query_as("SELECT generation, lease_id IS NULL FROM sync_queue WHERE id = $1")
            .bind(mutation.id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(state, (2, true));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn forced_flush_preserves_recorded_remote_success(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    sqlx::query(
        "INSERT INTO sync_queue (
             user_id, action_type, target_urn, next_run_at,
             remote_attempted_generation, remote_completed_generation, remote_result
         ) VALUES (
             '1', 'comment', 'track', now() + interval '1 hour', 1, 1, '{\"ok\":true}'
         )",
    )
    .execute(&pool)
    .await?;
    let repository = repository(pool.clone());

    repository.force_due().await?;

    let state: (i64, Option<i64>, bool) = sqlx::query_as(
        "SELECT generation, remote_completed_generation, remote_result IS NOT NULL
         FROM sync_queue",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(state, (1, Some(1), true));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn recorded_comment_is_finalized_without_another_remote_call(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    sqlx::query(
        "INSERT INTO sync_queue (user_id, action_type, target_urn, payload)
         VALUES ('1', 'comment', 'track', '{\"body\":\"hello\"}')",
    )
    .execute(&pool)
    .await?;
    let repository = repository(pool.clone());
    let mutation = repository.claim(1).await?.remove(0);
    assert!(repository.record_remote_attempt(&mutation).await?);
    repository
        .record_remote_success(&mutation, &json!({ "id": 1 }))
        .await?;

    repository.finalize(&mutation).await?;

    let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM sync_queue")
        .fetch_one(&pool)
        .await?;
    assert_eq!(remaining, 0);
    Ok(())
}

async fn seed_star_and_regular_writes(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "INSERT INTO soundcloud_connections (soundcloud_user_id) VALUES ('2');
         INSERT INTO subscriptions (user_urn, exp_date)
         VALUES ('2', extract(epoch FROM now())::bigint + 3600),
                ('1', extract(epoch FROM now())::bigint - 3600);
         INSERT INTO sync_queue (user_id, action_type, target_urn, created_at) VALUES
             ('1', 'like_track', 'old-like', now() - interval '10 minutes'),
             ('2', 'like_track', 'star-like', now() - interval '1 minute'),
             ('1', 'comment', 'old-comment', now() - interval '10 minutes'),
             ('2', 'comment', 'star-comment', now() - interval '1 minute')",
    )
    .execute(pool)
    .await?;
    Ok(())
}

fn claimed_targets(claimed: &[ClaimedMutation]) -> Vec<String> {
    let mut targets: Vec<String> = claimed
        .iter()
        .map(|mutation| mutation.target_urn.clone())
        .collect();
    targets.sort_unstable();
    targets
}

#[sqlx::test(migrations = false)]
async fn a_star_turn_sends_star_writes_first(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_star_and_regular_writes(&pool).await?;
    let repository = repository(pool);

    let claimed = repository.claim_outside(2, &[], true).await?;

    assert_eq!(claimed_targets(&claimed), ["star-comment", "star-like"]);
    assert!(claimed.iter().all(|mutation| mutation.star));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_regular_turn_sends_the_oldest_regular_writes_first(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_star_and_regular_writes(&pool).await?;
    let repository = repository(pool);

    let claimed = repository.claim_outside(2, &[], false).await?;

    assert_eq!(claimed_targets(&claimed), ["old-comment", "old-like"]);
    assert!(claimed.iter().all(|mutation| !mutation.star));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn the_writes_left_waiting_after_a_star_turn_show_up_per_group(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_star_and_regular_writes(&pool).await?;
    let repository = repository(pool);

    repository.claim_outside(2, &[], true).await?;
    let next = repository.next_waiting(&[]).await?;

    assert!(next.star.is_none());
    assert!(next.regular.is_some());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_fresh_like_goes_before_a_month_old_backlog_and_star_still_goes_first(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    sqlx::raw_sql(
        "INSERT INTO soundcloud_connections (soundcloud_user_id) VALUES ('2'), ('3');
         INSERT INTO subscriptions (user_urn, exp_date)
         VALUES ('3', extract(epoch FROM now())::bigint + 3600);
         INSERT INTO sync_queue (user_id, action_type, target_urn, created_at, next_run_at) VALUES
             ('1', 'like_track', 'backlog-like', now() - interval '30 days', now() - interval '30 days'),
             ('2', 'like_track', 'fresh-like', now(), now()),
             ('3', 'like_track', 'star-backlog-like', now() - interval '30 days', now() - interval '30 days')",
    )
    .execute(&pool)
    .await?;
    let repository = repository(pool);

    let star_turn = repository.claim_outside(8, &[], true).await?;
    let regular_turn = repository.claim_outside(8, &[], false).await?;
    let last_turn = repository.claim_outside(8, &[], false).await?;

    assert_eq!(claimed_targets(&star_turn), ["star-backlog-like"]);
    assert_eq!(claimed_targets(&regular_turn), ["fresh-like"]);
    assert_eq!(claimed_targets(&last_turn), ["backlog-like"]);
    Ok(())
}

async fn oauth_app(pool: &PgPool) -> anyhow::Result<Uuid> {
    let app = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO oauth_apps (id, name, client_id, client_secret, redirect_uri)
         VALUES ($1, 'test', $2, 'secret', 'https://app.test/')",
    )
    .bind(app)
    .bind(app.to_string())
    .execute(pool)
    .await?;
    Ok(app)
}

async fn connect(
    pool: &PgPool,
    app: Uuid,
    refreshed_minutes_ago: i32,
    error_kind: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO soundcloud_connections (
             id, soundcloud_user_id, oauth_app_id, access_token, refresh_token,
             expires_at, scope, last_refresh_error_kind, updated_at
         ) VALUES (
             $1, '17', $2, $3, 'refresh', now() + interval '1 day', '', $4,
             now() - $5::int * interval '1 minute'
         )",
    )
    .bind(Uuid::now_v7())
    .bind(app)
    .bind(app.to_string())
    .bind(error_kind)
    .bind(refreshed_minutes_ago)
    .execute(pool)
    .await?;
    Ok(())
}

async fn refuse_writes(pool: &PgPool, app: Uuid) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO sync_queue_action_pauses (oauth_app_id, action_type, paused_until, strikes, reason)
         VALUES ($1, 'playlist_create', now() - interval '1 hour', 1, 'subscription_required')",
    )
    .bind(app)
    .execute(pool)
    .await?;
    Ok(())
}

async fn write_app(pool: &PgPool) -> anyhow::Result<Option<Uuid>> {
    let client = TokenRefreshClient::new(&crate::config::OAuthConfig {
        token_url: "http://127.0.0.1:1/token".parse()?,
        bootstrap_app: None,
    })?;
    let token = ConnectionManager::new(pool.clone())
        .access_token(&client, "soundcloud:users:17")
        .await?;
    Ok(token.oauth_app_id)
}

#[sqlx::test(migrations = false)]
async fn a_saved_pause_keeps_its_reason_and_restores_with_a_strike(
    pool: PgPool,
) -> anyhow::Result<()> {
    crate::db::migrations::run_core(&pool, None).await?;
    let app = oauth_app(&pool).await?;
    let repository = repository(pool.clone());

    repository
        .save_action_pause(app, "like_track", 600, 1, "subscription_required")
        .await?;
    let restored = repository.action_pauses().await?;
    repository
        .save_action_pause(app, "like_track", 60, 2, "rate_limited")
        .await?;

    let reason: String =
        sqlx::query_scalar("SELECT reason FROM sync_queue_action_pauses WHERE oauth_app_id = $1")
            .bind(app)
            .fetch_one(&pool)
            .await?;
    assert_eq!(reason, "subscription_required");
    assert_eq!(restored.len(), 1);
    assert!(restored[0].2 > 590 && restored[0].2 <= 600);
    assert!(restored[0].3 >= 1);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_pause_saved_for_a_day_is_cut_to_two_hours(pool: PgPool) -> anyhow::Result<()> {
    crate::db::migrations::run_core(&pool, None).await?;
    let app = oauth_app(&pool).await?;
    let repository = repository(pool.clone());
    repository
        .save_action_pause(app, "like_track", 24 * 60 * 60, 9, "rate_limited")
        .await?;

    repository
        .cap_action_pauses(Duration::from_secs(2 * 60 * 60))
        .await?;

    let restored = repository.action_pauses().await?;
    assert!(restored[0].2 <= 2 * 60 * 60);
    assert_eq!(restored[0].3, 9);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn writes_go_through_the_good_app_even_when_the_refused_one_was_refreshed_later(
    pool: PgPool,
) -> anyhow::Result<()> {
    crate::db::migrations::run_core(&pool, None).await?;
    let good = oauth_app(&pool).await?;
    let refused = oauth_app(&pool).await?;
    connect(&pool, good, 60, None).await?;
    connect(&pool, refused, 0, None).await?;
    refuse_writes(&pool, refused).await?;
    sqlx::query("INSERT INTO sync_queue (user_id, action_type, target_urn) VALUES ('17', 'like_track', 'soundcloud:tracks:1')")
        .execute(&pool)
        .await?;
    let repository = repository(pool.clone());

    let behind_the_good_app = repository
        .claim_outside(8, &[format!("{good}:like_track")], true)
        .await?;
    let behind_the_refused_app = repository
        .claim_outside(8, &[format!("{refused}:like_track")], true)
        .await?;

    assert!(behind_the_good_app.is_empty());
    assert_eq!(behind_the_refused_app.len(), 1);
    assert_eq!(write_app(&pool).await?, Some(good));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_good_app_that_needs_sign_in_does_not_beat_a_usable_refused_one(
    pool: PgPool,
) -> anyhow::Result<()> {
    crate::db::migrations::run_core(&pool, None).await?;
    let good = oauth_app(&pool).await?;
    let refused = oauth_app(&pool).await?;
    connect(&pool, good, 0, Some("reauthorization_required")).await?;
    connect(&pool, refused, 60, None).await?;
    refuse_writes(&pool, refused).await?;

    assert_eq!(write_app(&pool).await?, Some(refused));
    Ok(())
}
