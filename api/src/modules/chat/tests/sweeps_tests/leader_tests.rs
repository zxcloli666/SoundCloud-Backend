use std::time::Duration;

use redis::AsyncCommands;

use super::support::{ok, redis_db};
use crate::modules::chat::rt_keys;
use crate::modules::chat::sweeps::leader::{hold, hold_as, keep, release_as};
use crate::modules::chat::test_support::chat_state;

const LEAD_DB: u8 = 11;

#[tokio::test]
#[ignore = "requires a local Redis"]
async fn one_process_leads_renews_and_hands_over_on_release() -> anyhow::Result<()> {
    let pg =
        sqlx::postgres::PgPoolOptions::new().connect_lazy("postgres://scd@127.0.0.1:1/none")?;
    let redis = crate::redis::connect_url(&redis_db(LEAD_DB))?;
    let chat = chat_state(pg, redis, true);
    let mut conn = chat.redis.get().await?;
    let _: i64 = conn.del(rt_keys::LEAD).await?;

    assert!(ok(hold_as(&chat, "first").await)?);
    assert!(
        !ok(hold_as(&chat, "second").await)?,
        "the lease is exclusive"
    );
    let _: bool = conn.pexpire(rt_keys::LEAD, 5_000).await?;
    assert!(ok(hold_as(&chat, "first").await)?, "the holder renews");
    let ttl: i64 = conn.pttl(rt_keys::LEAD).await?;
    assert!(
        ttl > 5_000 && ttl <= rt_keys::LEAD_TTL.as_millis() as i64,
        "{ttl}"
    );

    assert!(
        !ok(release_as(&chat, "second").await)?,
        "only the holder releases"
    );
    assert!(!ok(hold_as(&chat, "second").await)?);
    assert!(ok(release_as(&chat, "first").await)?);
    assert!(
        ok(hold_as(&chat, "second").await)?,
        "a released lease is taken at once"
    );
    assert!(ok(release_as(&chat, "second").await)?);

    let keeper = tokio::spawn(keep(chat.clone()));
    let mut taken = false;
    for _ in 0..100 {
        taken = conn.exists(rt_keys::LEAD).await?;
        if taken {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(taken, "the keeper takes the free lease");
    assert!(
        !ok(hold_as(&chat, "second").await)?,
        "and keeps it from others"
    );
    assert!(ok(hold(&chat).await)?, "this process holds it");
    chat.shutdown.cancel();
    keeper.await?;
    assert!(
        ok(hold_as(&chat, "second").await)?,
        "the keeper gives the lease back on shutdown"
    );
    let _: i64 = conn.del(rt_keys::LEAD).await?;
    Ok(())
}
