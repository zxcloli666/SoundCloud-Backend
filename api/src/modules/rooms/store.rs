use deadpool_redis::Pool;
use redis::{AsyncCommands, Script};

use crate::error::{AppError, AppResult};
use crate::modules::rooms::model::Room;

pub const ROOM_TTL_SECS: i64 = 6 * 60 * 60;
const CAS_ATTEMPTS: usize = 8;

const CREATE_SCRIPT: &str = r"
if redis.call('EXISTS', KEYS[1]) == 1 then return 0 end
redis.call('HSET', KEYS[1], 'v', ARGV[1], 'body', ARGV[2])
redis.call('EXPIRE', KEYS[1], ARGV[3])
return 1
";

const SWAP_SCRIPT: &str = r"
if redis.call('HGET', KEYS[1], 'v') ~= ARGV[1] then return 0 end
redis.call('HSET', KEYS[1], 'v', ARGV[2], 'body', ARGV[3])
redis.call('EXPIRE', KEYS[1], ARGV[4])
redis.call('EXPIRE', KEYS[2], ARGV[4])
return 1
";

pub struct RoomStore {
    redis: Pool,
}

fn room_key(code: &str) -> String {
    format!("rooms:{code}")
}

fn seen_key(code: &str) -> String {
    format!("rooms:{code}:seen")
}

fn encode(room: &Room) -> AppResult<String> {
    serde_json::to_string(room).map_err(|error| AppError::internal(format!("room encode: {error}")))
}

impl RoomStore {
    pub fn new(redis: Pool) -> Self {
        Self { redis }
    }

    pub async fn insert(&self, room: &Room) -> AppResult<bool> {
        let mut conn = self.redis.get().await?;
        let created: i64 = Script::new(CREATE_SCRIPT)
            .key(room_key(&room.code))
            .arg(room.version)
            .arg(encode(room)?)
            .arg(ROOM_TTL_SECS)
            .invoke_async(&mut conn)
            .await?;
        Ok(created == 1)
    }

    pub async fn load(&self, code: &str) -> AppResult<Option<Room>> {
        let mut conn = self.redis.get().await?;
        let body: Option<String> = conn.hget(room_key(code), "body").await?;
        body.map(|body| {
            serde_json::from_str(&body)
                .map_err(|error| AppError::internal(format!("room decode: {error}")))
        })
        .transpose()
    }

    pub async fn version(&self, code: &str) -> AppResult<Option<u64>> {
        let mut conn = self.redis.get().await?;
        Ok(conn.hget(room_key(code), "v").await?)
    }

    pub async fn update<T, F>(&self, code: &str, mut change: F) -> AppResult<(Room, T)>
    where
        F: FnMut(&mut Room) -> AppResult<T>,
    {
        for _ in 0..CAS_ATTEMPTS {
            let mut room = self
                .load(code)
                .await?
                .ok_or_else(|| AppError::not_found("Room not found"))?;
            let expected = room.version;
            let outcome = change(&mut room)?;
            room.version = expected + 1;
            let mut conn = self.redis.get().await?;
            let swapped: i64 = Script::new(SWAP_SCRIPT)
                .key(room_key(code))
                .key(seen_key(code))
                .arg(expected)
                .arg(room.version)
                .arg(encode(&room)?)
                .arg(ROOM_TTL_SECS)
                .invoke_async(&mut conn)
                .await?;
            if swapped == 1 {
                return Ok((room, outcome));
            }
        }
        Err(AppError::conflict(
            "The room changed too quickly, try again",
        ))
    }

    pub async fn delete(&self, code: &str) -> AppResult<()> {
        let mut conn = self.redis.get().await?;
        let _: i64 = conn.del(&[room_key(code), seen_key(code)]).await?;
        Ok(())
    }

    pub async fn touch(&self, code: &str, user_id: &str, now: i64) -> AppResult<()> {
        let key = seen_key(code);
        let mut conn = self.redis.get().await?;
        redis::pipe()
            .zadd(&key, user_id, now)
            .ignore()
            .expire(&key, ROOM_TTL_SECS)
            .ignore()
            .query_async::<()>(&mut conn)
            .await?;
        Ok(())
    }

    pub async fn forget(&self, code: &str, user_id: &str) -> AppResult<()> {
        let mut conn = self.redis.get().await?;
        let _: i64 = conn.zrem(seen_key(code), user_id).await?;
        Ok(())
    }

    pub async fn seen_since(&self, code: &str, since: i64) -> AppResult<Vec<String>> {
        let mut conn = self.redis.get().await?;
        Ok(conn.zrangebyscore(seen_key(code), since, "+inf").await?)
    }
}
