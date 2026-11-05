use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use sc_chat_wire::seal::SealedRequest;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{Device, call, error, exchange, from_device, redis, registered, seal, session_for};
use crate::common::session::session_hash;
use crate::modules::chat::caller::cache::COLD_LOOKUPS;
use crate::modules::chat::caller::{Cold, SessionEntry};
use crate::modules::chat::config::Access;
use crate::modules::chat::media::support::cancel_a_lock_wait;
use crate::modules::chat::sweeps::sessions::{self, PER_CALL, Pass};
use crate::modules::chat::synthetic::USER_PREFIX;
use crate::modules::chat::test_support::chat_state;
use crate::modules::chat::{ChatError, ChatState, routes};

struct User {
    device: Device,
    session: Uuid,
}

fn open_state(pool: &PgPool) -> anyhow::Result<Arc<ChatState>> {
    let Ok(mut state) = Arc::try_unwrap(chat_state(pool.clone(), redis()?, true)) else {
        panic!("a fresh chat state has one owner");
    };
    state.config.access = Access::All;
    Ok(Arc::new(state))
}

async fn users(pool: &PgPool, count: usize, prefix: &str) -> anyhow::Result<Vec<User>> {
    let mut made = Vec::with_capacity(count);
    for index in 0..count {
        let name = format!("{prefix}{index}");
        let device = Device::new();
        let session = session_for(pool, &name, Some(&device)).await?;
        registered(pool, &device.row(&name, session, "active")).await?;
        made.push(User { device, session });
    }
    Ok(made)
}

fn idle(user: &User) -> SealedRequest {
    seal(
        from_device(call("GET", "/chat/idle", ""), user.session, &user.device),
        &user.device.seed,
    )
}

async fn elsewhere_log_out(pool: &PgPool, session: Uuid) -> anyhow::Result<()> {
    let other = PgPoolOptions::new()
        .max_connections(1)
        .connect_with((*pool.connect_options()).clone())
        .await?;
    let ended: bool =
        sqlx::query_scalar("SELECT public.auth_session_end($1, sha256(uuid_send($1)))")
            .bind(session)
            .fetch_one(&other)
            .await?;
    other.close().await;
    anyhow::ensure!(ended, "the session was not ended");
    Ok(())
}

async fn fill_the_gate(
    chat: &Arc<ChatState>,
    release: &CancellationToken,
) -> anyhow::Result<JoinSet<()>> {
    let mut holders = JoinSet::new();
    for _ in 0..COLD_LOOKUPS {
        let (chat, release) = (chat.clone(), release.clone());
        holders.spawn(async move {
            let held = chat
                .callers
                .cold(Cold::Session, Duration::from_secs(10), async {
                    release.cancelled().await;
                    Ok(())
                });
            let _ = held.await;
        });
    }
    let deadline = Instant::now() + Duration::from_secs(5);
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

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_logout_in_another_process_stops_the_session_after_its_next_use(
    pool: PgPool,
) -> anyhow::Result<()> {
    let chat = open_state(&pool)?;
    let app: Router = routes::public(chat.clone());
    let both = users(&pool, 2, "logout-").await?;
    let (gone, kept) = (&both[0], &both[1]);
    for user in &both {
        assert_eq!(
            exchange(&app, &idle(user)).await,
            error(ChatError::NotFound)
        );
    }
    let kept_until = chat
        .callers
        .session(kept.session)
        .map(|held| held.fresh_until);
    sessions::check(&chat).await;

    elsewhere_log_out(&pool, gone.session).await?;
    assert_eq!(
        exchange(&app, &idle(gone)).await,
        error(ChatError::NotFound)
    );
    assert_eq!(
        exchange(&app, &idle(kept)).await,
        error(ChatError::NotFound)
    );
    let pass = sessions::check(&chat).await;

    assert_eq!(
        pass,
        Pass {
            calls: vec![2],
            evicted: 1,
            skipped: false
        }
    );
    assert!(chat.callers.session(gone.session).is_none());
    assert!(chat.devices.held_row(gone.device.id).is_none());
    assert_eq!(
        exchange(&app, &idle(gone)).await,
        error(ChatError::ReloginRequired)
    );
    assert_eq!(
        chat.callers
            .session(kept.session)
            .map(|held| held.fresh_until),
        kept_until
    );
    assert_eq!(
        exchange(&app, &idle(kept)).await,
        error(ChatError::NotFound)
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_skipped_or_failed_pass_evicts_nothing_and_the_next_one_checks_again(
    pool: PgPool,
) -> anyhow::Result<()> {
    let chat = open_state(&pool)?;
    let app: Router = routes::public(chat.clone());
    let one = users(&pool, 1, "logout-skip-").await?;
    let gone = &one[0];
    assert_eq!(
        exchange(&app, &idle(gone)).await,
        error(ChatError::NotFound)
    );
    sessions::check(&chat).await;
    elsewhere_log_out(&pool, gone.session).await?;
    assert_eq!(
        exchange(&app, &idle(gone)).await,
        error(ChatError::NotFound)
    );

    let release = CancellationToken::new();
    let mut holders = fill_the_gate(&chat, &release).await?;
    let busy = sessions::check(&chat).await;
    release.cancel();
    while holders.join_next().await.is_some() {}
    assert_eq!(
        busy,
        Pass {
            calls: Vec::new(),
            evicted: 0,
            skipped: true
        }
    );
    assert!(chat.callers.session(gone.session).is_some());

    let mut lock = pool.begin().await?;
    sqlx::query("LOCK TABLE public.sessions IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await?;
    let checking = {
        let chat = chat.clone();
        tokio::spawn(async move { sessions::check(&chat).await })
    };
    cancel_a_lock_wait(&pool, None).await?;
    let failed = checking.await?;
    lock.rollback().await?;
    assert_eq!(
        failed,
        Pass {
            calls: Vec::new(),
            evicted: 0,
            skipped: true
        }
    );
    assert!(chat.callers.session(gone.session).is_some());

    let next = sessions::check(&chat).await;
    assert_eq!((next.calls, next.evicted), (vec![1], 1));
    assert_eq!(
        exchange(&app, &idle(gone)).await,
        error(ChatError::ReloginRequired)
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_pass_checks_only_used_sessions_and_at_most_the_cap_per_call(
    pool: PgPool,
) -> anyhow::Result<()> {
    let chat = open_state(&pool)?;
    let ids: Vec<Uuid> = (0..3).map(|_| Uuid::now_v7()).collect();
    for id in &ids {
        chat.callers.remember_session(
            *id,
            SessionEntry {
                user_id: "cached".to_owned(),
                bind: None,
            },
        );
    }
    assert!(chat.callers.session(ids[0]).is_some());
    let used = sessions::check(&chat).await;
    assert_eq!((used.calls, used.evicted), (vec![1], 1));
    assert!(chat.callers.session(ids[1]).is_some());
    assert!(chat.callers.session(ids[2]).is_some());
    sessions::check(&chat).await;

    let many = 2 * PER_CALL + 5;
    chat.callers
        .mark_used((0..many).map(|_| session_hash(Uuid::now_v7())));
    let capped = sessions::check(&chat).await;
    assert_eq!(capped.calls, vec![PER_CALL, PER_CALL, 5]);
    assert!(!capped.skipped);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_pass_leaves_loadtest_sessions_that_postgres_never_knew(
    pool: PgPool,
) -> anyhow::Result<()> {
    let chat = open_state(&pool)?;
    let (loadtest, real) = (Uuid::now_v7(), Uuid::now_v7());
    for (id, user_id) in [
        (loadtest, format!("{USER_PREFIX}7")),
        (real, "cached".to_owned()),
    ] {
        chat.callers.remember_session(
            id,
            SessionEntry {
                user_id,
                bind: None,
            },
        );
        assert!(chat.callers.session(id).is_some());
    }
    let first = sessions::check(&chat).await;
    assert_eq!((first.calls, first.evicted), (vec![2], 1));
    assert!(chat.callers.session(real).is_none());
    assert!(chat.callers.session(loadtest).is_some());
    let second = sessions::check(&chat).await;
    assert_eq!((second.calls, second.evicted), (vec![1], 0));
    assert!(chat.callers.session(loadtest).is_some());
    Ok(())
}
