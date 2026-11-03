use redis::Script;
use serde::{Deserialize, Serialize};

use crate::error::AppResult;
use crate::modules::chat::b64;
use crate::modules::rooms::store::RoomStore;

pub const PAGE_MAX: usize = 1_000;

const TAKE_SCRIPT: &str = r"
local out = {}
for _, key in ipairs(KEYS) do
  local dump = redis.call('DUMP', key)
  local pttl = redis.call('PTTL', key)
  if dump and pttl ~= -2 then
    out[#out + 1] = key
    out[#out + 1] = dump
    out[#out + 1] = tostring(pttl)
  end
  redis.call('UNLINK', key)
end
return out
";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Page {
    pub cursor: String,
    pub keys: Vec<Moved>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Moved {
    pub key: String,
    #[serde(with = "b64")]
    pub dump: Vec<u8>,
    pub pttl: i64,
}

impl RoomStore {
    pub async fn take_page(&self, cursor: &str, count: usize) -> AppResult<Page> {
        let mut conn = self.redis.get().await?;
        let (next, keys): (String, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg("rooms:*")
            .arg("COUNT")
            .arg(count.clamp(1, PAGE_MAX))
            .query_async(&mut conn)
            .await?;
        if keys.is_empty() {
            return Ok(Page {
                cursor: next,
                keys: Vec::new(),
            });
        }
        let script = Script::new(TAKE_SCRIPT);
        let mut call = script.prepare_invoke();
        for key in &keys {
            call.key(key);
        }
        let flat: Vec<Vec<u8>> = call.invoke_async(&mut conn).await?;
        Ok(Page {
            cursor: next,
            keys: flat
                .as_chunks::<3>()
                .0
                .iter()
                .map(|[key, dump, pttl]| Moved {
                    key: String::from_utf8_lossy(key).into_owned(),
                    dump: dump.clone(),
                    pttl: String::from_utf8_lossy(pttl).parse().unwrap_or(0),
                })
                .collect(),
        })
    }

    pub async fn put_page(&self, moved: &[Moved]) -> AppResult<usize> {
        if moved.is_empty() {
            return Ok(0);
        }
        let mut pipe = redis::pipe();
        for room in moved {
            pipe.cmd("RESTORE")
                .arg(&room.key)
                .arg(room.pttl.max(0))
                .arg(room.dump.as_slice())
                .arg("REPLACE")
                .ignore();
        }
        let mut conn = self.redis.get().await?;
        pipe.query_async::<()>(&mut conn).await?;
        Ok(moved.len())
    }
}
