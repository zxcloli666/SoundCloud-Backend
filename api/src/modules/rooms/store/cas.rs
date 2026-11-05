use redis::Script;

use super::{MOVED_TTL_SECS, ROOM_TTL_SECS, RoomStore, encode, moved_key, room_key, seen_key};
use crate::error::{AppError, AppResult};
use crate::modules::rooms::model::{Room, new_code};

const CAS_ATTEMPTS: usize = 8;

const SWAP_SCRIPT: &str = r"
if redis.call('HGET', KEYS[1], 'v') ~= ARGV[1] then return 0 end
if ARGV[5] ~= '' and redis.call('GET', KEYS[3]) ~= ARGV[5] then return -1 end
redis.call('HSET', KEYS[1], 'v', ARGV[2], 'body', ARGV[3])
redis.call('EXPIRE', KEYS[1], ARGV[4])
redis.call('EXPIRE', KEYS[2], ARGV[4])
return 1
";

const MOVE_SCRIPT: &str = r"
if redis.call('HGET', KEYS[1], 'v') ~= ARGV[1] then return 0 end
if redis.call('EXISTS', KEYS[2]) == 1 then return 0 end
redis.call('HSET', KEYS[2], 'v', ARGV[2], 'body', ARGV[3])
redis.call('EXPIRE', KEYS[2], ARGV[4])
if redis.call('EXISTS', KEYS[3]) == 1 then
  redis.call('RENAME', KEYS[3], KEYS[4])
  redis.call('EXPIRE', KEYS[4], ARGV[4])
end
redis.call('DEL', KEYS[1])
redis.call('SET', KEYS[5], ARGV[5], 'EX', ARGV[6])
return 1
";

pub struct Guard<'a> {
    pub key: &'a str,
    pub token: &'a str,
}

impl RoomStore {
    pub async fn update<T, F>(
        &self,
        code: &str,
        guard: Option<&Guard<'_>>,
        mut change: F,
    ) -> AppResult<(Room, T)>
    where
        F: FnMut(&mut Room) -> AppResult<T>,
    {
        let room_key = room_key(code);
        let (guard_key, token) =
            guard.map_or((room_key.as_str(), ""), |guard| (guard.key, guard.token));
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
                .key(room_key.as_str())
                .key(seen_key(code))
                .key(guard_key)
                .arg(expected)
                .arg(room.version)
                .arg(encode(&room)?)
                .arg(ROOM_TTL_SECS)
                .arg(token)
                .invoke_async(&mut conn)
                .await?;
            match swapped {
                1 => return Ok((room, outcome)),
                -1 => return Err(AppError::conflict("The room changed elsewhere, try again")),
                _ => {}
            }
        }
        Err(AppError::conflict(
            "The room changed too quickly, try again",
        ))
    }

    pub async fn relocate<F>(&self, code: &str, mut change: F) -> AppResult<Room>
    where
        F: FnMut(&mut Room) -> AppResult<()>,
    {
        for _ in 0..CAS_ATTEMPTS {
            let mut room = self
                .load(code)
                .await?
                .ok_or_else(|| AppError::not_found("Room not found"))?;
            let expected = room.version;
            change(&mut room)?;
            room.code = new_code();
            room.version = expected + 1;
            let mut conn = self.redis.get().await?;
            let moved: i64 = Script::new(MOVE_SCRIPT)
                .key(room_key(code))
                .key(room_key(&room.code))
                .key(seen_key(code))
                .key(seen_key(&room.code))
                .key(moved_key(code))
                .arg(expected)
                .arg(room.version)
                .arg(encode(&room)?)
                .arg(ROOM_TTL_SECS)
                .arg(&room.code)
                .arg(MOVED_TTL_SECS)
                .invoke_async(&mut conn)
                .await?;
            if moved == 1 {
                return Ok(room);
            }
        }
        Err(AppError::conflict(
            "The room changed too quickly, try again",
        ))
    }
}
