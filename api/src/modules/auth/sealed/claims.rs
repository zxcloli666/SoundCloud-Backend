use std::time::Duration;

use deadpool_redis::Pool;
use sha2::{Digest, Sha256};

use crate::modules::chat::{ChatError, ChatResult};

pub const ENC_TTL: Duration = Duration::from_secs(300);
pub const SOLUTION_TTL: Duration = Duration::from_secs(600);

pub fn enc_key(enc: &[u8]) -> String {
    let digest = Sha256::digest(enc);
    format!("auth:x:enc:{}", hex::encode(&digest[..16]))
}

pub fn solution_key(seed: &[u8], nonce: &[u8], auth_pub: &[u8]) -> String {
    let digest = Sha256::new()
        .chain_update(seed)
        .chain_update(nonce)
        .chain_update(auth_pub)
        .finalize();
    format!("auth:x:pow:{}", hex::encode(digest))
}

pub async fn claim(redis: &Pool, key: String, ttl: Duration) -> ChatResult<()> {
    let mut redis = redis.get().await?;
    let fresh: Option<String> = redis::cmd("SET")
        .arg(key)
        .arg(1)
        .arg("NX")
        .arg("EX")
        .arg(ttl.as_secs())
        .query_async(&mut *redis)
        .await?;
    match fresh {
        Some(_) => Ok(()),
        None => Err(ChatError::Replay),
    }
}
