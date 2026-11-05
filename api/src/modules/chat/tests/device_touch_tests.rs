use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use sqlx::PgPool;
use tokio::task::JoinSet;
use uuid::Uuid;

use crate::modules::chat::devices::test_support::state;
use crate::modules::chat::devices::{lookup, revoke, touch, touch_after};
use crate::modules::chat::held::FRESH_SPREAD;
use crate::modules::chat::model::DeviceRow;
use crate::modules::chat::test_support::chat_state;
use crate::modules::chat::{ChatError, ChatState};

const WAVE: i64 = 200;
const FILLED: i64 = 2_000;
const KEPT_FOR: Duration = Duration::from_secs(600);
const LOCK_WAIT: Duration = Duration::from_secs(5);

fn offline_redis_state(pool: &PgPool) -> Arc<ChatState> {
    let redis = crate::redis::connect_url("redis://127.0.0.1:1").expect("a lazy redis pool");
    chat_state(pool.clone(), redis, true)
}

async fn devices_seen_ago(pool: &PgPool, count: i64, minutes: i32) -> anyhow::Result<Vec<Uuid>> {
    Ok(sqlx::query_scalar(
        "INSERT INTO chat.devices
            (id, user_id, session_hash, auth_pub, box_pub, platform, name, app_version, tier, last_seen_at)
         SELECT gen_random_uuid(), 'wave-' || g, sha256(uuid_send(gen_random_uuid())),
                '\\x01'::bytea, '\\x02'::bytea, 'macos', 'MacBook', '8.7.0', 'direct',
                now() - $2::int * interval '1 minute'
         FROM generate_series(1, $1) AS g
         RETURNING id",
    )
    .bind(count)
    .bind(minutes)
    .fetch_all(pool)
    .await?)
}

async fn cached(chat: &ChatState, ids: &[Uuid]) -> anyhow::Result<Vec<DeviceRow>> {
    let mut rows = Vec::with_capacity(ids.len());
    for id in ids {
        rows.push(
            lookup(chat, *id)
                .await
                .expect("a lookup")
                .expect("a device"),
        );
    }
    Ok(rows)
}

async fn one_idle(chat: &ChatState, pool: &PgPool) -> anyhow::Result<DeviceRow> {
    let ids = devices_seen_ago(pool, 1, 20).await?;
    Ok(cached(chat, &ids).await?.remove(0))
}

async fn a_query_waits_on_a_row_lock(pool: &PgPool) -> anyhow::Result<()> {
    let deadline = Instant::now() + LOCK_WAIT;
    loop {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity
             WHERE datname = current_database() AND wait_event_type = 'Lock'",
        )
        .fetch_one(pool)
        .await?;
        if waiting > 0 {
            return Ok(());
        }
        anyhow::ensure!(Instant::now() < deadline, "the touch never reached the row");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn spawn_touch(
    chat: &Arc<ChatState>,
    row: &DeviceRow,
) -> tokio::task::JoinHandle<Result<(), ChatError>> {
    let chat = chat.clone();
    let row = row.clone();
    tokio::spawn(async move { touch(&chat, &row).await })
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_touch_wave_leaves_every_device_cached(pool: PgPool) -> anyhow::Result<()> {
    let chat = offline_redis_state(&pool);
    let idle = cached(&chat, &devices_seen_ago(&pool, WAVE, 20).await?).await?;
    let readers = devices_seen_ago(&pool, WAVE, 0).await?;

    let mut wave = JoinSet::new();
    for row in idle.clone() {
        let chat = chat.clone();
        wave.spawn(async move { touch(&chat, &row).await.map(|()| None) });
    }
    for id in readers.clone() {
        let chat = chat.clone();
        wave.spawn(async move { lookup(&chat, id).await });
    }
    while let Some(done) = wave.join_next().await {
        done?.expect("a touch or a lookup");
    }

    let cold: Vec<Uuid> = idle
        .iter()
        .map(|row| row.id)
        .chain(readers.iter().copied())
        .filter(|id| chat.devices.row(*id).is_none())
        .collect();
    assert_eq!(
        cold.len(),
        0,
        "{} of {} devices went cold after the touch wave",
        cold.len(),
        idle.len() + readers.len()
    );
    for before in &idle {
        let after = chat.devices.row(before.id).expect("a kept row");
        assert!(after.last_seen_at > before.last_seen_at);
    }
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn device_rows_cached_together_go_stale_over_minutes(pool: PgPool) -> anyhow::Result<()> {
    let chat = offline_redis_state(&pool);
    let ids = devices_seen_ago(&pool, FILLED, 0).await?;
    let filled = Instant::now();
    cached(&chat, &ids).await?;
    let done = Instant::now();

    let earliest = filled + KEPT_FOR.mul_f64(1.0 - FRESH_SPREAD);
    let mut per_second: HashMap<u64, usize> = HashMap::new();
    for id in &ids {
        let held = chat.devices.held_row(*id).expect("a held row");
        assert!(held.fresh());
        assert!(held.fresh_until >= earliest && held.fresh_until <= done + KEPT_FOR);
        *per_second
            .entry(held.fresh_until.duration_since(filled).as_secs())
            .or_default() += 1;
    }
    let busiest = per_second.values().copied().max().unwrap_or_default();
    assert!(per_second.len() >= 100, "{} seconds", per_second.len());
    assert!(
        busiest <= 50,
        "{busiest} of {FILLED} rows go stale in one second"
    );
    Ok(())
}

#[test]
fn devices_seen_together_touch_over_five_minutes() {
    let mut per_second: HashMap<i64, usize> = HashMap::new();
    for _ in 0..10_000 {
        let after = touch_after(Uuid::new_v4());
        assert!(after >= chrono::Duration::minutes(10) && after < chrono::Duration::minutes(15));
        *per_second.entry(after.num_seconds()).or_default() += 1;
    }
    let busiest = per_second.values().copied().max().unwrap_or_default();
    assert!(per_second.len() >= 290, "{} seconds", per_second.len());
    assert!(busiest <= 100, "{busiest} touches in one second");
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_touch_that_waited_on_a_revoke_does_not_bring_the_device_back(
    pool: PgPool,
) -> anyhow::Result<()> {
    let chat = offline_redis_state(&pool);
    let row = one_idle(&chat, &pool).await?;
    let mut revoking = pool.begin().await?;
    sqlx::query("UPDATE chat.devices SET status = 'revoked', revoked_at = now() WHERE id = $1")
        .bind(row.id)
        .execute(&mut *revoking)
        .await?;

    let touching = spawn_touch(&chat, &row);
    a_query_waits_on_a_row_lock(&pool).await?;
    revoking.commit().await?;
    touching.await?.expect("a touch");

    assert!(
        chat.devices.held_row(row.id).is_none(),
        "a touch that matched no active row must drop the cached one"
    );
    let after = lookup(&chat, row.id).await.expect("a lookup");
    assert_eq!(after.map(|row| row.status), Some("revoked".to_owned()));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_forget_during_a_touch_is_not_undone_by_it(pool: PgPool) -> anyhow::Result<()> {
    let chat = offline_redis_state(&pool);
    let row = one_idle(&chat, &pool).await?;
    let mut holding = pool.begin().await?;
    sqlx::query("SELECT 1 FROM chat.devices WHERE id = $1 FOR UPDATE")
        .bind(row.id)
        .execute(&mut *holding)
        .await?;

    let touching = spawn_touch(&chat, &row);
    a_query_waits_on_a_row_lock(&pool).await?;
    chat.devices.forget(&row.user_id, &[row.id]);
    holding.rollback().await?;
    touching.await?.expect("a touch");

    assert!(
        chat.devices.held_row(row.id).is_none(),
        "a touch must not keep a row read across a forget"
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_revoke_after_a_touch_drops_the_touched_row(pool: PgPool) -> anyhow::Result<()> {
    let chat = state(&pool);
    let row = one_idle(&chat, &pool).await?;
    touch(&chat, &row).await.expect("a touch");
    let touched = chat.devices.row(row.id).expect("a kept row");

    assert_eq!(revoke(&chat, &touched).await, Ok(true));
    assert!(chat.devices.held_row(row.id).is_none());
    let after = lookup(&chat, row.id).await.expect("a lookup");
    assert_eq!(after.map(|row| row.status), Some("revoked".to_owned()));
    Ok(())
}
