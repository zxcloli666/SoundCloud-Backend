use std::sync::LazyLock;
use std::time::Duration;

use redis::Script;
use uuid::Uuid;

use crate::modules::chat::{ChatError, ChatResult, ChatState, rt_keys};
use crate::modules::rooms::store::Guard;

const LOCK_MS: u64 = 15_000;
const LOCK_TRIES: usize = 60;
const LOCK_PAUSE: Duration = Duration::from_millis(50);

static KEEP: LazyLock<Script> = LazyLock::new(|| {
    Script::new(
        r"
if redis.call('GET', KEYS[1]) == ARGV[1] then
    return redis.call('PEXPIRE', KEYS[1], ARGV[2])
end
return 0
",
    )
});

static UNLOCK: LazyLock<Script> = LazyLock::new(|| {
    Script::new(
        r"
if redis.call('GET', KEYS[1]) == ARGV[1] then
    return redis.call('DEL', KEYS[1])
end
return 0
",
    )
});

pub struct Held {
    key: String,
    token: String,
}

impl Held {
    pub async fn take(chat: &ChatState, conversation: Uuid) -> ChatResult<Self> {
        let key = rt_keys::backdrop_lock(conversation);
        let token = Uuid::new_v4().to_string();
        for _ in 0..LOCK_TRIES {
            let mut conn = chat.redis.get().await?;
            let taken: Option<String> = redis::cmd("SET")
                .arg(&key)
                .arg(&token)
                .arg("NX")
                .arg("PX")
                .arg(LOCK_MS)
                .query_async(&mut conn)
                .await?;
            if taken.is_some() {
                return Ok(Self { key, token });
            }
            drop(conn);
            tokio::time::sleep(LOCK_PAUSE).await;
        }
        Err(ChatError::busy())
    }

    pub fn guard(&self) -> Guard<'_> {
        Guard {
            key: &self.key,
            token: &self.token,
        }
    }

    pub async fn kept(&self, chat: &ChatState) -> bool {
        let kept = async {
            let mut conn = chat.redis.get().await?;
            let kept: i64 = KEEP
                .key(&self.key)
                .arg(&self.token)
                .arg(LOCK_MS)
                .invoke_async(&mut conn)
                .await?;
            ChatResult::Ok(kept == 1)
        };
        match kept.await {
            Ok(kept) => kept,
            Err(error) => {
                tracing::warn!(
                    code = error.code(),
                    "a room backdrop lock could not be kept"
                );
                false
            }
        }
    }

    pub async fn release(self, chat: &ChatState) {
        let released = async {
            let mut conn = chat.redis.get().await?;
            UNLOCK
                .key(&self.key)
                .arg(&self.token)
                .invoke_async::<i64>(&mut conn)
                .await?;
            ChatResult::Ok(())
        };
        if let Err(error) = released.await {
            tracing::warn!(
                code = error.code(),
                "a room backdrop lock waits for its ttl"
            );
        }
    }
}
