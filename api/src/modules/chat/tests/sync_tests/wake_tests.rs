use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::test_helpers::{
    Answer, caller, conversation, device, messages, settle, sync, user, with_postgres,
};
use crate::modules::chat::ChatState;
use crate::modules::chat::caller::Cold;
use crate::modules::chat::caller::cache::COLD_LOOKUPS;
use crate::modules::chat::devices::{self, DeviceCache};
use crate::modules::chat::sync::bump;

const HERD: usize = 48;
const HOLD_MS: u64 = 10_000;
const ANSWER_WAIT: Duration = Duration::from_secs(8);

struct Known {
    user: String,
    device: Uuid,
    pos: u64,
    inc: Uuid,
}

struct Herd {
    chat: Arc<ChatState>,
    room: Uuid,
    users: Vec<String>,
    known: Vec<Known>,
}

async fn herd(pool: &PgPool) -> anyhow::Result<Herd> {
    let Ok(mut state) = Arc::try_unwrap(with_postgres(pool)) else {
        anyhow::bail!("a fresh chat state has one owner");
    };
    state.devices = DeviceCache::with_freshness(Duration::ZERO);
    let chat = Arc::new(state);
    let users: Vec<String> = (0..HERD).map(|_| user()).collect();
    let names: Vec<&str> = users.iter().map(String::as_str).collect();
    let room = conversation(pool, "group", &names).await;
    let mut known = Vec::with_capacity(HERD);
    for user in &users {
        let device = device(pool, user, "active").await;
        let first = sync(&chat, caller(user, device), json!({"max_bytes": 262_144})).await;
        anyhow::ensure!(first.status == StatusCode::OK, "{}", first.body);
        devices::lookup(&chat, device)
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        known.push(Known {
            user: user.clone(),
            device,
            pos: first.body["pos"].as_u64().unwrap_or_default(),
            inc: serde_json::from_value(first.body["inc"].clone())?,
        });
    }
    Ok(Herd {
        chat,
        room,
        users,
        known,
    })
}

async fn fill_the_gate(
    chat: &Arc<ChatState>,
    release: &CancellationToken,
) -> anyhow::Result<JoinSet<()>> {
    let mut holders = JoinSet::new();
    for _ in 0..COLD_LOOKUPS {
        let (chat, release) = (chat.clone(), release.clone());
        holders.spawn(async move {
            let held = chat.callers.cold(Cold::Device, ANSWER_WAIT, async {
                release.cancelled().await;
                Ok(())
            });
            let _ = held.await;
        });
    }
    let deadline = Instant::now() + ANSWER_WAIT;
    while chat
        .callers
        .cold(Cold::Device, Duration::ZERO, async { Ok(()) })
        .await
        .is_ok()
    {
        anyhow::ensure!(Instant::now() < deadline, "the cold gate never filled");
        tokio::task::yield_now().await;
    }
    Ok(holders)
}

async fn lock_devices(pool: &PgPool) -> anyhow::Result<Transaction<'static, Postgres>> {
    let mut lock = pool.begin().await?;
    sqlx::query("LOCK TABLE chat.devices IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await?;
    Ok(lock)
}

async fn device_waits(pool: &PgPool) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT count(*) FROM pg_locks WHERE relation = 'chat.devices'::regclass AND NOT granted",
    )
    .fetch_one(pool)
    .await?)
}

fn spawn_polls(herd: &Herd, body: impl Fn(&Known) -> Value) -> JoinSet<Answer> {
    let mut polls = JoinSet::new();
    for known in &herd.known {
        let (chat, caller, body) = (
            herd.chat.clone(),
            caller(&known.user, known.device),
            body(known),
        );
        polls.spawn(async move { sync(&chat, caller, body).await });
    }
    polls
}

async fn answers(mut polls: JoinSet<Answer>) -> anyhow::Result<Vec<Answer>> {
    tokio::time::timeout(ANSWER_WAIT, async {
        let mut all = Vec::with_capacity(HERD);
        while let Some(done) = polls.join_next().await {
            all.push(done?);
        }
        anyhow::Ok(all)
    })
    .await?
}

fn counted(name: &str, label: &str) -> f64 {
    crate::metrics::render_for_tests()
        .unwrap_or_default()
        .lines()
        .filter(|line| line.starts_with(name) && line.contains(label))
        .filter_map(|line| line.rsplit(' ').next()?.parse::<f64>().ok())
        .sum()
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_mass_wake_against_a_full_cold_gate_reads_no_device_row_and_answers_every_device(
    pool: PgPool,
) -> anyhow::Result<()> {
    crate::metrics::init();
    let herd = herd(&pool).await?;
    messages(&pool, herd.room, &herd.users[0], 1, 16).await;
    let release = CancellationToken::new();
    let mut holders = fill_the_gate(&herd.chat, &release).await?;
    let stale_before = counted("chat_auth_stale_total", "kind=\"device\"");
    let polls = spawn_polls(&herd, |known| {
        json!({
            "pos": known.pos, "inc": known.inc, "hold_ms": HOLD_MS,
            "max_bytes": 262_144, "known": [[herd.room, 0]]
        })
    });
    settle(&herd.chat, HERD).await;

    let lock = lock_devices(&pool).await?;
    bump::conversation(&herd.chat, herd.room, 1, &herd.users)
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    let woke = answers(polls).await;
    let waits = device_waits(&pool).await?;
    lock.rollback().await?;
    let stale = counted("chat_auth_stale_total", "kind=\"device\"") - stale_before;
    release.cancel();
    while holders.join_next().await.is_some() {}

    assert_eq!(waits, 0, "a woken poll read chat.devices");
    let woke = woke?;
    assert_eq!(woke.len(), HERD);
    for answer in &woke {
        assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
        assert_eq!(answer.body["convs"][0]["id"], json!(herd.room));
    }
    assert!(stale >= HERD as f64, "{stale} stale answers");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_mass_reset_against_a_full_cold_gate_reads_no_device_list_and_answers_every_user(
    pool: PgPool,
) -> anyhow::Result<()> {
    let herd = herd(&pool).await?;
    let release = CancellationToken::new();
    let mut holders = fill_the_gate(&herd.chat, &release).await?;

    let lock = lock_devices(&pool).await?;
    let polls = spawn_polls(
        &herd,
        |known| json!({"pos": known.pos, "inc": Uuid::now_v7(), "hold_ms": HOLD_MS, "max_bytes": 262_144}),
    );
    let reset = answers(polls).await;
    let waits = device_waits(&pool).await?;
    lock.rollback().await?;
    release.cancel();
    while holders.join_next().await.is_some() {}

    assert_eq!(waits, 0, "a reset read chat.devices");
    let reset = reset?;
    assert_eq!(reset.len(), HERD);
    for answer in &reset {
        assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
        assert_eq!(answer.body["reset"], true);
        assert_eq!(answer.body["devices"].as_array().map(Vec::len), Some(1));
    }
    Ok(())
}
