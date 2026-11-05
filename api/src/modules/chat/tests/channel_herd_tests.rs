use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use sc_chat_wire::seal::SealedRequest;
use sqlx::PgPool;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{Device, call, error, exchange, from_device, redis, registered, seal, session_for};
use crate::modules::chat::caller::cache::COLD_LOOKUPS;
use crate::modules::chat::caller::{Callers, Cold};
use crate::modules::chat::config::Access;
use crate::modules::chat::devices::DeviceCache;
use crate::modules::chat::test_support::chat_state;
use crate::modules::chat::{ChatError, ChatState, routes};

const KNOWN: usize = 24;
const STRANGERS: usize = 8;
const GATE_FILL: Duration = Duration::from_secs(5);

struct Herder {
    device: Device,
    session: Uuid,
}

fn stale_state(pool: &PgPool) -> anyhow::Result<Arc<ChatState>> {
    let Ok(mut state) = Arc::try_unwrap(chat_state(pool.clone(), redis()?, true)) else {
        panic!("a fresh chat state has one owner");
    };
    state.config.access = Access::All;
    state.callers = Callers::with_freshness(Duration::ZERO);
    state.devices = DeviceCache::with_freshness(Duration::ZERO);
    Ok(Arc::new(state))
}

async fn herders(pool: &PgPool, count: usize, prefix: &str) -> anyhow::Result<Vec<Herder>> {
    let mut made = Vec::with_capacity(count);
    for index in 0..count {
        let user = format!("{prefix}{index}");
        let device = Device::new();
        let session = session_for(pool, &user, Some(&device)).await?;
        registered(pool, &device.row(&user, session, "active")).await?;
        made.push(Herder { device, session });
    }
    Ok(made)
}

fn idle(herder: &Herder) -> SealedRequest {
    seal(
        from_device(
            call("GET", "/chat/idle", ""),
            herder.session,
            &herder.device,
        ),
        &herder.device.seed,
    )
}

async fn fill_the_gate(
    chat: &Arc<ChatState>,
    release: &CancellationToken,
) -> anyhow::Result<JoinSet<()>> {
    let mut holders = JoinSet::new();
    for _ in 0..COLD_LOOKUPS {
        let chat = chat.clone();
        let release = release.clone();
        holders.spawn(async move {
            let held = chat.callers.cold(Cold::Session, GATE_FILL, async {
                release.cancelled().await;
                Ok(())
            });
            let _ = held.await;
        });
    }
    let deadline = Instant::now() + GATE_FILL;
    while chat
        .callers
        .cold(Cold::Session, Duration::ZERO, async { Ok(()) })
        .await
        .is_ok()
    {
        anyhow::ensure!(Instant::now() < deadline, "the cold gate never filled");
        tokio::task::yield_now().await;
    }
    Ok(holders)
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
async fn a_full_cold_gate_answers_every_known_caller_and_sheds_only_new_ones(
    pool: PgPool,
) -> anyhow::Result<()> {
    crate::metrics::init();
    let chat = stale_state(&pool)?;
    let app: Router = routes::public(chat.clone());
    let known = herders(&pool, KNOWN, "herd-known-").await?;
    let strangers = herders(&pool, STRANGERS, "herd-new-").await?;
    for herder in &known {
        assert_eq!(
            exchange(&app, &idle(herder)).await,
            error(ChatError::NotFound)
        );
    }

    let release = CancellationToken::new();
    let mut holders = fill_the_gate(&chat, &release).await?;
    let shed_before = counted("chat_busy_total", "source=\"cold_session\"");
    let stale_before = counted("chat_auth_stale_total", "kind=\"device\"");
    let mut herd = JoinSet::new();
    for (index, herder) in known.iter().chain(&strangers).enumerate() {
        let app = app.clone();
        let request = idle(herder);
        herd.spawn(async move { (index, exchange(&app, &request).await) });
    }
    let mut answers = vec![None; KNOWN + STRANGERS];
    while let Some(done) = herd.join_next().await {
        let (index, answer) = done?;
        answers[index] = Some(answer);
    }
    let shed = counted("chat_busy_total", "source=\"cold_session\"") - shed_before;
    let stale = counted("chat_auth_stale_total", "kind=\"device\"") - stale_before;
    release.cancel();
    while holders.join_next().await.is_some() {}

    for answer in &answers[..KNOWN] {
        assert_eq!(answer.as_ref(), Some(&error(ChatError::NotFound)));
    }
    for answer in &answers[KNOWN..] {
        let (status, body) = answer.as_ref().expect("an answer");
        assert_eq!((*status, body["error"].as_str()), (503, Some("chat_busy")));
        let retry = body["retry_ms"].as_u64().unwrap_or_default();
        assert!((1_000..3_000).contains(&retry), "{body}");
    }
    assert!(shed >= STRANGERS as f64, "{shed} shed");
    assert!(stale >= KNOWN as f64, "{stale} stale answers");

    for herder in &strangers {
        assert_eq!(
            exchange(&app, &idle(herder)).await,
            error(ChatError::NotFound)
        );
    }
    Ok(())
}

fn bounded_state(pool: &PgPool) -> anyhow::Result<Arc<ChatState>> {
    let Ok(mut state) = Arc::try_unwrap(chat_state(pool.clone(), redis()?, true)) else {
        panic!("a fresh chat state has one owner");
    };
    state.config.access = Access::All;
    state.callers = Callers::with_ages(Duration::ZERO, Duration::ZERO);
    state.devices = DeviceCache::with_ages(Duration::ZERO, Duration::ZERO);
    Ok(Arc::new(state))
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_caller_held_past_the_stale_bound_is_busy_not_served_from_the_cache(
    pool: PgPool,
) -> anyhow::Result<()> {
    let chat = bounded_state(&pool)?;
    let app: Router = routes::public(chat.clone());
    let known = herders(&pool, 4, "herd-bounded-").await?;
    for herder in &known {
        assert_eq!(
            exchange(&app, &idle(herder)).await,
            error(ChatError::NotFound)
        );
    }

    let release = CancellationToken::new();
    let mut holders = fill_the_gate(&chat, &release).await?;
    let mut answers = Vec::new();
    for herder in &known {
        answers.push(exchange(&app, &idle(herder)).await);
    }
    release.cancel();
    while holders.join_next().await.is_some() {}

    for (status, body) in &answers {
        assert_eq!((*status, body["error"].as_str()), (503, Some("chat_busy")));
    }
    for herder in &known {
        assert_eq!(
            exchange(&app, &idle(herder)).await,
            error(ChatError::NotFound)
        );
    }
    Ok(())
}
