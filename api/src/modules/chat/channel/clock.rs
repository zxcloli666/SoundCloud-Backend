use chrono::Utc;

use crate::modules::chat::{ChatError, ChatResult};

pub const CLOCK_SKEW_MS: i64 = 120_000;

pub fn check(ts_ms: u64) -> ChatResult<()> {
    let server_now = Utc::now().timestamp_millis();
    let sent = i64::try_from(ts_ms).unwrap_or(i64::MAX);
    if sent.abs_diff(server_now) > CLOCK_SKEW_MS.unsigned_abs() {
        return Err(ChatError::Clock { server_now });
    }
    Ok(())
}
