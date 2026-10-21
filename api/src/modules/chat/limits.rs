use std::sync::LazyLock;
use std::time::Duration;

use deadpool_redis::redis::Script;
use uuid::Uuid;

use crate::modules::chat::{ChatError, ChatResult, ChatState, rt_keys};

const SECOND: Duration = Duration::from_secs(1);
const MINUTE: Duration = Duration::from_secs(60);
const HOUR: Duration = Duration::from_secs(3_600);
const DAY: Duration = Duration::from_secs(86_400);

pub const ROOM_NEWCOMER_SECS: i64 = 600;
pub const ROOM_NEWCOMER_BODY_MAX: usize = chat_wire::seal::NONCE_LEN + 16 + 2 * 256;
pub const ROOM_BYTES_PER_MINUTE: u64 = 24 * 1024;

const SCRIPT: &str = r#"
for i = 1, #KEYS do
    local base = (i - 1) * 3
    local limit = tonumber(ARGV[base + 1])
    local cost = tonumber(ARGV[base + 3])
    local used = tonumber(redis.call('GET', KEYS[i]) or '0')
    if used + cost > limit then
        local ttl = redis.call('PTTL', KEYS[i])
        if ttl < 0 then
            ttl = tonumber(ARGV[base + 2])
        end
        return math.max(ttl, 1)
    end
end
for i = 1, #KEYS do
    local base = (i - 1) * 3
    redis.call('INCRBY', KEYS[i], ARGV[base + 3])
    if redis.call('PTTL', KEYS[i]) < 0 then
        redis.call('PEXPIRE', KEYS[i], ARGV[base + 2])
    end
end
return 0
"#;

static ADMIT: LazyLock<Script> = LazyLock::new(|| Script::new(SCRIPT));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bucket {
    Send,
    Create,
    Request,
    Invite,
    Report,
    Device,
    ReadMessages,
    ReadList,
    ReadKeys,
    ReadCard,
    Preview,
}

impl Bucket {
    pub fn name(self) -> &'static str {
        match self {
            Bucket::Send => "send",
            Bucket::Create => "create",
            Bucket::Request => "request",
            Bucket::Invite => "invite",
            Bucket::Report => "report",
            Bucket::Device => "device",
            Bucket::ReadMessages => "read_messages",
            Bucket::ReadList => "read_list",
            Bucket::ReadKeys => "read_keys",
            Bucket::ReadCard => "read_card",
            Bucket::Preview => "preview",
        }
    }

    pub fn windows(self) -> &'static [Window] {
        match self {
            Bucket::Send => &SEND_WINDOWS,
            Bucket::Create | Bucket::Invite => &HOURLY_30,
            Bucket::Request | Bucket::Report => &DAILY_20,
            Bucket::Device => &DAILY_5,
            Bucket::ReadMessages | Bucket::ReadKeys | Bucket::ReadCard => &MINUTE_60,
            Bucket::ReadList => &MINUTE_10,
            Bucket::Preview => &MINUTE_30,
        }
    }

    fn rejection(self) -> ChatError {
        match self {
            Bucket::Send => ChatError::SendLimited,
            Bucket::ReadMessages
            | Bucket::ReadList
            | Bucket::ReadKeys
            | Bucket::ReadCard
            | Bucket::Preview => ChatError::ReadLimited,
            Bucket::Create | Bucket::Request | Bucket::Invite | Bucket::Report | Bucket::Device => {
                ChatError::RequestLimit
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub label: &'static str,
    pub limit: u64,
    pub length: Duration,
}

impl Window {
    pub const fn new(label: &'static str, limit: u64, length: Duration) -> Self {
        Self {
            label,
            limit,
            length,
        }
    }
}

const SEND_WINDOWS: [Window; 3] = [
    Window::new("20s", 10, Duration::from_secs(20)),
    Window::new("1m", 30, MINUTE),
    Window::new("1d", 2_000, DAY),
];
const HOURLY_30: [Window; 1] = [Window::new("1h", 30, HOUR)];
const DAILY_20: [Window; 1] = [Window::new("1d", 20, DAY)];
const DAILY_5: [Window; 1] = [Window::new("1d", 5, DAY)];
const MINUTE_60: [Window; 1] = [Window::new("1m", 60, MINUTE)];
const MINUTE_30: [Window; 1] = [Window::new("1m", 30, MINUTE)];
const MINUTE_10: [Window; 1] = [Window::new("1m", 10, MINUTE)];
const NEWCOMER_REQUESTS: Window = Window::new("1d:new", 10, DAY);
const PER_CONVERSATION_SENDS: Window = Window::new("5s", 5, Duration::from_secs(5));

struct Charge {
    key: String,
    limit: u64,
    window_ms: u64,
    cost: u64,
}

impl Charge {
    fn of(bucket: &str, window: &Window, user_id: &str, cost: u64) -> Self {
        Self {
            key: rt_keys::limit(&format!("{bucket}:{}", window.label), user_id),
            limit: window.limit,
            window_ms: window.length.as_millis() as u64,
            cost,
        }
    }
}

pub async fn admit(chat: &ChatState, bucket: Bucket, user_id: &str) -> ChatResult<()> {
    let charges = bucket
        .windows()
        .iter()
        .map(|window| Charge::of(bucket.name(), window, user_id, 1))
        .collect();
    charge(chat, bucket.name(), charges, bucket.rejection()).await
}

pub async fn admit_send(chat: &ChatState, user_id: &str, conversation: Uuid) -> ChatResult<()> {
    let mut charges: Vec<Charge> = Bucket::Send
        .windows()
        .iter()
        .map(|window| Charge::of(Bucket::Send.name(), window, user_id, 1))
        .collect();
    charges.push(Charge::of(
        &format!("send:{conversation}"),
        &PER_CONVERSATION_SENDS,
        user_id,
        1,
    ));
    charge(chat, Bucket::Send.name(), charges, ChatError::SendLimited).await
}

pub async fn admit_request(chat: &ChatState, user_id: &str, newcomer: bool) -> ChatResult<()> {
    let bucket = Bucket::Request;
    let mut charges: Vec<Charge> = bucket
        .windows()
        .iter()
        .map(|window| Charge::of(bucket.name(), window, user_id, 1))
        .collect();
    if newcomer {
        charges.push(Charge::of(bucket.name(), &NEWCOMER_REQUESTS, user_id, 1));
    }
    charge(chat, bucket.name(), charges, ChatError::RequestLimit).await
}

pub struct RoomPost {
    pub conversation: Uuid,
    pub public: bool,
    pub member_for_secs: i64,
    pub body_len: usize,
}

pub async fn admit_room_post(chat: &ChatState, user_id: &str, post: &RoomPost) -> ChatResult<()> {
    if !post.public {
        return Ok(());
    }
    if post.member_for_secs < ROOM_NEWCOMER_SECS && post.body_len > ROOM_NEWCOMER_BODY_MAX {
        metrics::counter!("chat_limit_rejects_total", "bucket" => "room_newcomer").increment(1);
        return Err(ChatError::SendLimited);
    }
    let budget = Window::new("bytes:1m", ROOM_BYTES_PER_MINUTE, MINUTE);
    let charges = vec![Charge::of(
        &format!("room:{}", post.conversation),
        &budget,
        user_id,
        post.body_len as u64,
    )];
    charge(chat, "room_bytes", charges, ChatError::SendLimited).await
}

async fn charge(
    chat: &ChatState,
    label: &'static str,
    charges: Vec<Charge>,
    rejection: ChatError,
) -> ChatResult<()> {
    let mut invocation = ADMIT.prepare_invoke();
    for charge in &charges {
        invocation
            .key(&charge.key)
            .arg(charge.limit)
            .arg(charge.window_ms.max(SECOND.as_millis() as u64))
            .arg(charge.cost);
    }
    let mut connection = chat.redis.get().await?;
    let retry_ms: u64 = invocation.invoke_async(&mut connection).await?;
    if retry_ms == 0 {
        return Ok(());
    }
    metrics::counter!("chat_limit_rejects_total", "bucket" => label).increment(1);
    Err(rejection)
}

#[cfg(test)]
#[path = "tests/limits_tests.rs"]
mod tests;
