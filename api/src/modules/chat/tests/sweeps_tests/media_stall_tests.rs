use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{Duration as Span, Utc};
use sqlx::PgPool;
use tokio::net::TcpListener;

use super::support::{conversation, redis_db, user};
use crate::modules::chat::ChatState;
use crate::modules::chat::media::MediaConfig;
use crate::modules::chat::quota;
use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::sweeps::media::fixtures::{
    keys_dir, ok, owner_delete, state_with, upload,
};
use crate::modules::chat::sweeps::media::{delete, reconcile};

const STALL_DB: u8 = 7;

async fn stalled_store() -> anyhow::Result<String> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });
    Ok(format!("http://{address}"))
}

fn stalled_state(pg: PgPool, endpoint: &str) -> Arc<ChatState> {
    let dir = keys_dir(&[(1, "stalled-secret")]);
    let media = MediaConfig::parse(
        |name| match name {
            "CHAT_STORES" | "CHAT_STORE_WRITE" => Some("1".to_owned()),
            "CHAT_STORE_1_ENDPOINT" | "CHAT_STORE_1_PRESIGN" => Some(endpoint.to_owned()),
            "CHAT_STORE_1_BUCKET" => Some("stalled".to_owned()),
            "CHAT_STORE_1_ACCESS_KEY" => Some("stalled".to_owned()),
            _ => None,
        },
        &dir,
    )
    .expect("a media config");
    let redis = crate::redis::connect_url(&redis_db(STALL_DB)).expect("a redis pool");
    state_with(pg, redis, media)
}

async fn claimed(pg: &PgPool) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT count(*) FROM chat.media WHERE status = 'deleting' AND lease_until > now()",
    )
    .fetch_one(pg)
    .await?)
}

async fn until_claimed(pg: &PgPool, rows: i64) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while claimed(pg).await? < rows {
        anyhow::ensure!(
            Instant::now() < deadline,
            "the delete step never claimed its rows"
        );
        tokio::task::yield_now().await;
    }
    Ok(())
}

async fn pool_settles(chat: &ChatState) -> bool {
    let deadline = Instant::now() + Duration::from_millis(250);
    while chat.pg.size() as usize > chat.pg.num_idle() {
        if Instant::now() >= deadline {
            return false;
        }
        tokio::task::yield_now().await;
    }
    true
}

async fn watch_pool(chat: &ChatState, owner: &str, window: Duration) -> anyhow::Result<u32> {
    let until = Instant::now() + window;
    let mut reads = 0;
    while Instant::now() < until {
        assert!(
            pool_settles(chat).await,
            "a PG connection is held while the store stalls"
        );
        let asked = Instant::now();
        let used = tokio::time::timeout(Duration::from_secs(1), quota::used(chat, owner)).await;
        assert!(
            used.is_ok_and(|used| used.is_ok()),
            "a chat read did not answer"
        );
        assert!(asked.elapsed() < Duration::from_secs(1));
        reads += 1;
    }
    Ok(reads)
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_stalled_store_holds_no_pg_connection_and_chat_reads_stay_fast(
    pg: PgPool,
) -> anyhow::Result<()> {
    let endpoint = stalled_store().await?;
    let chat = stalled_state(pg.clone(), &endpoint);
    let owner = user();
    let group = conversation(&pg, "group", &owner).await?;
    for _ in 0..20 {
        let image = upload(&pg, &owner, group, 1).await?;
        owner_delete(&pg, image.id, -1).await?;
    }
    let mut conn = chat.redis.get().await?;
    let _: () = redis::cmd("DEL")
        .arg(reconcile::cursor::KEY)
        .query_async(&mut conn)
        .await?;
    drop(conn);

    let deleting = {
        let chat = chat.clone();
        tokio::spawn(async move { delete::run(&chat, &mut Budget::new(delete::CLAIMS)).await })
    };
    until_claimed(&pg, 40).await?;
    assert!(watch_pool(&chat, &owner, Duration::from_secs(4)).await? > 0);
    assert!(!deleting.is_finished(), "the store is still stalled");
    deleting.abort();
    assert_eq!(
        claimed(&pg).await?,
        40,
        "the killed step left its lease for the next leader"
    );

    let now = Utc::now()
        .date_naive()
        .and_hms_opt(2, 0, 0)
        .expect("a time")
        .and_utc()
        + Span::days(1);
    let reconciling = {
        let chat = chat.clone();
        tokio::spawn(async move { reconcile::run(&chat, now, &mut Budget::new(5)).await })
    };
    assert!(watch_pool(&chat, &owner, Duration::from_secs(4)).await? > 0);
    assert!(!reconciling.is_finished(), "the LIST is still stalled");
    reconciling.abort();
    let saved = ok(reconcile::cursor::load(&chat).await)?;
    assert!(saved.is_none(), "nothing advanced, nothing was saved");
    Ok(())
}
