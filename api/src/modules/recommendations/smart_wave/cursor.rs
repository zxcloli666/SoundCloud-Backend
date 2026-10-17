use std::collections::VecDeque;

use deadpool_redis::Pool as RedisPool;
use deadpool_redis::redis::{AsyncCommands, pipe};
use rand::Rng;
use serde::{Deserialize, Serialize};
use tracing::debug;
use uuid::Uuid;

pub const TTL_SECS: u64 = 6 * 60 * 60;
const SEEN_TRACKS_CAP: usize = 200;
const ARTIST_WINDOW: usize = 30;
const NEG_WINDOW: u8 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SeedKind {
    User,
    Track,
    Artist,
}

impl SeedKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Track => "track",
            Self::Artist => "artist",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaveCursor {
    pub handle: String,
    pub seed_kind: SeedKind,
    pub seed_key: String,
    pub served: u32,
    pub seen_tracks: VecDeque<u64>,
    pub seen_artists: VecDeque<Uuid>,
    pub neg_flags: VecDeque<u8>,
    #[serde(default)]
    pub history_until: Option<i64>,
}

impl WaveCursor {
    pub fn new(seed_kind: SeedKind, seed_key: String) -> Self {
        let handle = random_handle();
        Self {
            handle,
            seed_kind,
            seed_key,
            served: 0,
            seen_tracks: VecDeque::with_capacity(SEEN_TRACKS_CAP),
            seen_artists: VecDeque::with_capacity(ARTIST_WINDOW),
            neg_flags: VecDeque::with_capacity(NEG_WINDOW as usize),
            history_until: None,
        }
    }

    pub fn redis_key(&self, owner: &str) -> String {
        format!(
            "smartwave:cursor:{}:{}:{}:{}",
            self.seed_kind.as_str(),
            owner,
            self.seed_key,
            self.handle
        )
    }

    pub fn mark_served(&mut self, sc_track_id: u64, artist_id: Option<Uuid>) {
        self.served = self.served.saturating_add(1);
        push_capped(&mut self.seen_tracks, sc_track_id, SEEN_TRACKS_CAP);
        if let Some(a) = artist_id {
            push_capped(&mut self.seen_artists, a, ARTIST_WINDOW);
        }
    }

    pub fn record_outcomes(&mut self, negatives: usize, positives: usize) {
        let (negatives, positives) = within_window(negatives, positives);
        for _ in 0..negatives {
            push_capped(&mut self.neg_flags, 1, NEG_WINDOW as usize);
        }
        for _ in 0..positives {
            push_capped(&mut self.neg_flags, 0, NEG_WINDOW as usize);
        }
    }

    pub fn neg_rate(&self) -> f32 {
        if self.neg_flags.is_empty() {
            return 0.0;
        }
        let negatives = self.neg_flags.iter().filter(|flag| **flag == 1).count();
        negatives as f32 / self.neg_flags.len() as f32
    }

    pub fn artist_count_in_window(&self, artist_id: Uuid) -> usize {
        self.seen_artists
            .iter()
            .filter(|a| **a == artist_id)
            .count()
    }

    pub fn contains(&self, sc_track_id: u64) -> bool {
        self.seen_tracks.iter().any(|t| *t == sc_track_id)
    }
}

fn within_window(negatives: usize, positives: usize) -> (usize, usize) {
    let window = NEG_WINDOW as usize;
    let total = negatives.saturating_add(positives);
    if total > window {
        let kept = (negatives as f64 / total as f64 * window as f64).round() as usize;
        (kept, window - kept)
    } else {
        (negatives, positives)
    }
}

fn push_capped<T>(buf: &mut VecDeque<T>, v: T, cap: usize) {
    if buf.len() >= cap {
        buf.pop_front();
    }
    buf.push_back(v);
}

fn random_handle() -> String {
    let mut rng = rand::thread_rng();
    let n: u64 = rng.r#gen();
    format!("{:x}", n & 0xffff_ffff)
}

pub async fn load_or_new(
    redis: &RedisPool,
    owner: &str,
    token: Option<&str>,
    seed_kind: SeedKind,
    seed_key: &str,
) -> WaveCursor {
    if let Some(t) = token
        && let Some(mut c) = read(redis, owner, t).await
        && c.seed_kind == seed_kind
        && c.seed_key == seed_key
    {
        let outcomes = outcomes_key(owner, t);
        let pending = match redis.get().await {
            Ok(mut conn) => pipe()
                .atomic()
                .hget(&outcomes, &["negatives", "positives"])
                .del(&outcomes)
                .ignore()
                .query_async::<((Option<usize>, Option<usize>),)>(&mut conn)
                .await
                .ok(),
            Err(_) => None,
        };
        if let Some(((negatives, positives),)) = pending {
            c.record_outcomes(negatives.unwrap_or(0), positives.unwrap_or(0));
        }
        return c;
    }
    WaveCursor::new(seed_kind, seed_key.to_string())
}

pub async fn save(redis: &RedisPool, owner: &str, cursor: &WaveCursor) -> Option<String> {
    let Ok(payload) = serde_json::to_string(cursor) else {
        return None;
    };
    let key = cursor.redis_key(owner);
    let Ok(mut conn) = redis.get().await else {
        debug!("smartwave: redis unavailable, cursor stateless");
        return None;
    };
    let res: Result<(), _> = conn.set_ex::<_, _, ()>(&key, payload, TTL_SECS).await;
    if let Err(e) = res {
        debug!(error = %e, "smartwave: cursor save failed");
        return None;
    }
    Some(cursor.handle.clone())
}

pub async fn read(redis: &RedisPool, owner: &str, handle: &str) -> Option<WaveCursor> {
    let mut conn = redis.get().await.ok()?;
    let backref: Option<String> = conn
        .get(handle_lookup_key(owner, handle))
        .await
        .ok()
        .flatten();
    let full_key = backref?;
    let raw: Option<String> = conn.get(&full_key).await.ok().flatten();
    let raw = raw?;
    serde_json::from_str(&raw).ok()
}

pub async fn register_handle(redis: &RedisPool, owner: &str, cursor: &WaveCursor) {
    let Ok(mut conn) = redis.get().await else {
        return;
    };
    let _: Result<(), _> = conn
        .set_ex::<_, _, ()>(
            handle_lookup_key(owner, &cursor.handle),
            cursor.redis_key(owner),
            TTL_SECS,
        )
        .await;
}

pub async fn touch(
    redis: &RedisPool,
    owner: &str,
    handle: &str,
    negatives: usize,
    positives: usize,
) -> bool {
    let Ok(mut conn) = redis.get().await else {
        return false;
    };
    let lookup = handle_lookup_key(owner, handle);
    let Ok(Some(full_key)) = conn.get::<_, Option<String>>(&lookup).await else {
        return false;
    };
    let ttl = TTL_SECS as i64;
    let (negatives, positives) = within_window(negatives, positives);
    let outcomes = outcomes_key(owner, handle);
    let mut touches = pipe();
    touches.expire(&full_key, ttl).expire(&lookup, ttl).ignore();
    if negatives + positives > 0 {
        touches
            .hincr(&outcomes, "negatives", negatives)
            .ignore()
            .hincr(&outcomes, "positives", positives)
            .ignore()
            .expire(&outcomes, ttl)
            .ignore();
    }
    touches
        .query_async::<(bool,)>(&mut conn)
        .await
        .is_ok_and(|(alive,)| alive)
}

fn handle_lookup_key(owner: &str, handle: &str) -> String {
    format!("smartwave:handle:{owner}:{handle}")
}

fn outcomes_key(owner: &str, handle: &str) -> String {
    format!("smartwave:outcomes:{owner}:{handle}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_huge_feedback_batch_keeps_its_share_inside_the_window() {
        let mut cursor = WaveCursor::new(SeedKind::User, "17".into());

        cursor.record_outcomes(3_000_000_000, 1_000_000_000);

        assert_eq!(cursor.neg_flags.len(), NEG_WINDOW as usize);
        assert!((cursor.neg_rate() - 0.75).abs() < 1e-6);
    }

    #[test]
    fn a_cursor_saved_by_the_previous_api_still_loads() -> anyhow::Result<()> {
        let saved = r#"{"handle":"ab","seed_kind":"Track","seed_key":"t7","served":1,
            "seen_tracks":[70],"seen_artists":[],"neg_flags":[1]}"#;

        let cursor: WaveCursor = serde_json::from_str(saved)?;

        assert!(cursor.contains(70));
        assert_eq!(cursor.history_until, None);
        Ok(())
    }

    #[test]
    fn the_negative_rate_reads_only_the_latest_outcomes() {
        let mut cursor = WaveCursor::new(SeedKind::Track, "t7".into());
        assert_eq!(cursor.neg_rate(), 0.0);

        cursor.record_outcomes(20, 0);
        cursor.record_outcomes(0, 10);

        assert!((cursor.neg_rate() - 0.5).abs() < 1e-6);
    }
}
