use deadpool_redis::Pool;
use redis::{AsyncCommands, Script};

use crate::error::AppResult;
use crate::modules::rooms::model::Room;
use crate::modules::rooms::store::ROOM_TTL_SECS;

const PUBLIC_KEY: &str = "rooms:public";

const RELEASE_SCRIPT: &str = r"
if redis.call('GET', KEYS[1]) == ARGV[1] then return redis.call('DEL', KEYS[1]) end
return 0
";

pub struct RoomDirectory {
    redis: Pool,
}

fn host_key(user_id: &str) -> String {
    format!("rooms:host:{user_id}")
}

impl RoomDirectory {
    pub fn new(redis: Pool) -> Self {
        Self { redis }
    }

    pub async fn hosted_by(&self, user_id: &str) -> AppResult<Option<String>> {
        let mut conn = self.redis.get().await?;
        Ok(conn.get(host_key(user_id)).await?)
    }

    pub async fn host_seen(&self, room: &Room, now: i64) -> AppResult<()> {
        let mut pipe = redis::pipe();
        pipe.set_ex(host_key(&room.host_id), &room.code, ROOM_TTL_SECS as u64)
            .ignore();
        if room.public && !room.unlisted {
            pipe.zadd(PUBLIC_KEY, &room.code, now)
                .ignore()
                .expire(PUBLIC_KEY, ROOM_TTL_SECS)
                .ignore();
        }
        let mut conn = self.redis.get().await?;
        pipe.query_async::<()>(&mut conn).await?;
        Ok(())
    }

    pub async fn unlist(&self, code: &str) -> AppResult<()> {
        let mut conn = self.redis.get().await?;
        let _: i64 = conn.zrem(PUBLIC_KEY, code).await?;
        Ok(())
    }

    pub async fn close(&self, room: &Room) -> AppResult<()> {
        self.unlist(&room.code).await?;
        self.release_host(&room.host_id, &room.code).await
    }

    pub async fn release_host(&self, user_id: &str, code: &str) -> AppResult<()> {
        let mut conn = self.redis.get().await?;
        let _: i64 = Script::new(RELEASE_SCRIPT)
            .key(host_key(user_id))
            .arg(code)
            .invoke_async(&mut conn)
            .await?;
        Ok(())
    }

    pub async fn listed_since(&self, since: i64, limit: isize) -> AppResult<Vec<String>> {
        let mut conn = self.redis.get().await?;
        let _: i64 = conn
            .zrembyscore(PUBLIC_KEY, "-inf", format!("({since}"))
            .await?;
        Ok(conn
            .zrevrangebyscore_limit(PUBLIC_KEY, "+inf", since, 0, limit)
            .await?)
    }
}
