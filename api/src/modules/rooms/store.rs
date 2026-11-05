mod cas;

use deadpool_redis::Pool;
use redis::{AsyncCommands, Script};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::modules::chat::rt_keys::ROOMS_ALIVE;
use crate::modules::rooms::model::Room;

pub const ROOM_TTL_SECS: i64 = 6 * 60 * 60;
const MOVED_TTL_SECS: i64 = 10 * 60;

const CREATE_SCRIPT: &str = r"
if redis.call('EXISTS', KEYS[1]) == 1 then return 0 end
redis.call('HSET', KEYS[1], 'v', ARGV[1], 'body', ARGV[2])
redis.call('EXPIRE', KEYS[1], ARGV[3])
return 1
";

pub use cas::Guard;

pub struct RoomStore {
    redis: Pool,
}

fn room_key(code: &str) -> String {
    format!("rooms:{code}")
}

fn seen_key(code: &str) -> String {
    format!("rooms:{code}:seen")
}

fn moved_key(code: &str) -> String {
    format!("rooms:{code}:moved")
}

fn chat_key(chat_id: Uuid) -> String {
    format!("rooms:chat:{chat_id}")
}

fn cooldown_key(host: &str) -> String {
    format!("rooms:host:{host}:backdrop_cooldown")
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

    pub async fn load_many(&self, codes: &[String]) -> AppResult<Vec<Room>> {
        if codes.is_empty() {
            return Ok(Vec::new());
        }
        let mut bodies = redis::pipe();
        for code in codes {
            bodies.hget(room_key(code), "body");
        }
        let mut conn = self.redis.get().await?;
        let bodies: Vec<Option<String>> = bodies.query_async(&mut conn).await?;
        Ok(bodies
            .into_iter()
            .filter_map(|body| serde_json::from_str(&body?).ok())
            .collect())
    }

    pub async fn alive_since(&self, since: i64) -> AppResult<Vec<String>> {
        let mut conn = self.redis.get().await?;
        Ok(conn.zrangebyscore(ROOMS_ALIVE, since, "+inf").await?)
    }

    pub async fn load_with_presence(
        &self,
        codes: &[String],
        since: i64,
    ) -> AppResult<Vec<(Room, Vec<String>)>> {
        if codes.is_empty() {
            return Ok(Vec::new());
        }
        let mut bodies = redis::pipe();
        let mut presence = redis::pipe();
        for code in codes {
            bodies.hget(room_key(code), "body");
            presence.zrangebyscore(seen_key(code), since, "+inf");
        }
        let mut conn = self.redis.get().await?;
        let bodies: Vec<Option<String>> = bodies.query_async(&mut conn).await?;
        let presence: Vec<Vec<String>> = presence.query_async(&mut conn).await?;
        Ok(bodies
            .into_iter()
            .zip(presence)
            .filter_map(|(body, online)| Some((serde_json::from_str(&body?).ok()?, online)))
            .collect())
    }

    pub async fn version(&self, code: &str) -> AppResult<Option<u64>> {
        let mut conn = self.redis.get().await?;
        Ok(conn.hget(room_key(code), "v").await?)
    }

    pub async fn moved_to(&self, code: &str) -> AppResult<Option<String>> {
        let mut conn = self.redis.get().await?;
        Ok(conn.get(moved_key(code)).await?)
    }

    pub async fn forget_room(&self, room: &Room) -> AppResult<()> {
        let mut pipe = redis::pipe();
        pipe.del(&[room_key(&room.code), seen_key(&room.code)])
            .ignore()
            .zrem(ROOMS_ALIVE, &room.code)
            .ignore();
        if let Some(chat_id) = room.chat_id {
            pipe.del(chat_key(chat_id)).ignore();
        }
        let mut conn = self.redis.get().await?;
        pipe.query_async::<()>(&mut conn).await?;
        Ok(())
    }

    pub async fn keep_alive(&self, room: &Room, now: i64) -> AppResult<()> {
        let mut pipe = redis::pipe();
        let forgotten = format!("({}", now - ROOM_TTL_SECS * 1_000);
        pipe.expire(room_key(&room.code), ROOM_TTL_SECS)
            .ignore()
            .zadd(ROOMS_ALIVE, &room.code, now)
            .ignore()
            .zrembyscore(ROOMS_ALIVE, "-inf", forgotten)
            .ignore()
            .expire(ROOMS_ALIVE, ROOM_TTL_SECS)
            .ignore();
        if let Some(chat_id) = room.chat_id {
            pipe.set_ex(chat_key(chat_id), &room.code, ROOM_TTL_SECS as u64)
                .ignore();
        }
        let mut conn = self.redis.get().await?;
        pipe.query_async::<()>(&mut conn).await?;
        Ok(())
    }

    pub async fn forget_alive(&self, code: &str) -> AppResult<()> {
        let mut conn = self.redis.get().await?;
        let _: i64 = conn.zrem(ROOMS_ALIVE, code).await?;
        Ok(())
    }

    pub async fn index_chat(&self, chat_id: Uuid, code: &str) -> AppResult<()> {
        let mut conn = self.redis.get().await?;
        conn.set_ex::<_, _, ()>(chat_key(chat_id), code, ROOM_TTL_SECS as u64)
            .await?;
        Ok(())
    }

    pub async fn chat_code(&self, chat_id: Uuid) -> AppResult<Option<String>> {
        let mut conn = self.redis.get().await?;
        Ok(conn.get(chat_key(chat_id)).await?)
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

    pub async fn cool_down(&self, host: &str, until: i64, now: i64) -> AppResult<()> {
        let left = u64::try_from(until - now).unwrap_or(0).max(1);
        let mut conn = self.redis.get().await?;
        redis::cmd("SET")
            .arg(cooldown_key(host))
            .arg(until)
            .arg("PX")
            .arg(left)
            .query_async::<()>(&mut conn)
            .await?;
        Ok(())
    }

    pub async fn cooling_until(&self, host: &str, now: i64) -> AppResult<Option<i64>> {
        let mut conn = self.redis.get().await?;
        let until: Option<i64> = conn.get(cooldown_key(host)).await?;
        Ok(until.filter(|until| *until > now))
    }
}
