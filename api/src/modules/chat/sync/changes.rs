use std::sync::LazyLock;

use redis::Script;
use uuid::Uuid;

use crate::modules::chat::rt_keys;
use crate::modules::chat::{ChatError, ChatResult, ChatState};

static CHANGES: LazyLock<Script> = LazyLock::new(|| Script::new(include_str!("changes.lua")));

const DEVICES: &str = "devices";
const REQUESTS: &str = "requests";
const GONE_MESSAGES: &str = "gone:";
const REQUESTS_FIELD: &str = "rq";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Conversation(Uuid),
    Devices,
    Requests,
    GoneMessages(Uuid, Vec<i64>),
}

impl Change {
    pub fn member(&self) -> String {
        match self {
            Change::Conversation(id) => id.to_string(),
            Change::Devices => DEVICES.to_owned(),
            Change::Requests => REQUESTS.to_owned(),
            Change::GoneMessages(id, seqs) => {
                let seqs: Vec<String> = seqs.iter().map(i64::to_string).collect();
                format!("{GONE_MESSAGES}{id}:{}", seqs.join(","))
            }
        }
    }

    pub fn parse(member: &str) -> Option<Self> {
        match member {
            DEVICES => Some(Change::Devices),
            REQUESTS => Some(Change::Requests),
            _ => match member.strip_prefix(GONE_MESSAGES) {
                Some(rest) => {
                    let (id, seqs) = rest.split_once(':')?;
                    let seqs = seqs.split(',').map(str::parse).collect::<Result<_, _>>();
                    Some(Change::GoneMessages(id.parse().ok()?, seqs.ok()?))
                }
                None => member.parse().ok().map(Change::Conversation),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Versions {
    pub version: u64,
    pub floor: u64,
    pub requests: Option<u32>,
    pub fresh: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delta {
    pub version: u64,
    pub change: Change,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Changes {
    pub versions: Versions,
    pub deltas: Vec<Delta>,
}

pub async fn read(chat: &ChatState, user_id: &str, pos: u64) -> ChatResult<Changes> {
    let mut conn = chat.redis.get().await?;
    let reply: Vec<String> = CHANGES
        .key(rt_keys::version(user_id))
        .key(rt_keys::deltas(user_id))
        .arg(pos)
        .arg(rt_keys::VERSION_TTL.as_secs())
        .invoke_async(&mut conn)
        .await?;
    decode(&reply).ok_or_else(|| {
        tracing::error!(user_id, "chat versions are malformed");
        ChatError::Internal
    })
}

pub async fn remember_requests(chat: &ChatState, user_id: &str, count: u32) -> ChatResult<()> {
    let key = rt_keys::version(user_id);
    let mut conn = chat.redis.get().await?;
    let _: () = redis::pipe()
        .hset(&key, REQUESTS_FIELD, count)
        .ignore()
        .expire(&key, rt_keys::VERSION_TTL.as_secs() as i64)
        .ignore()
        .query_async(&mut conn)
        .await?;
    Ok(())
}

pub fn decode(reply: &[String]) -> Option<Changes> {
    let [version, floor, requests, fresh, rest @ ..] = reply else {
        return None;
    };
    let versions = Versions {
        version: version.parse().ok()?,
        floor: floor.parse().ok()?,
        requests: requests.parse().ok(),
        fresh: fresh == "1",
    };
    let (pairs, _) = rest.as_chunks::<2>();
    let deltas = pairs
        .iter()
        .filter_map(|[member, version]| {
            Some(Delta {
                version: version.parse().ok()?,
                change: Change::parse(member)?,
            })
        })
        .collect();
    Some(Changes { versions, deltas })
}
