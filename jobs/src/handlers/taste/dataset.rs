use std::collections::BTreeMap;

use backend_contracts::pipeline::TASTE_DATASET_VERSION;
use backend_contracts::vector_store::{
    TRACKS_CLAP, TRACKS_CLAP_DIMENSIONS, TRACKS_COLLAB, TRACKS_COLLAB_DIMENSIONS, TRACKS_MERT,
    TRACKS_MERT_DIMENSIONS,
};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::Utc;
use futures::TryStreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tempfile::TempPath;
use tokio::io::{AsyncWriteExt, BufWriter};
use uuid::Uuid;

use crate::qdrant::QdrantProvisioner;
use crate::queue::{JobError, JobResult};

use super::history::{EventKind, HistoryGrouper, UserHistory, event_of};

const TESTABLE_TIMED_POSITIVES: usize = 5;
const FEATURE_PAGE: u32 = 256;
const PSEUDONYM_BYTES: usize = 16;
const SHA256_BLOCK: usize = 64;
const SECONDS_PER_DAY: i64 = 86_400;

#[derive(sqlx::FromRow)]
pub(super) struct EventRow {
    pub user_id: String,
    pub track_id: i64,
    pub event_code: i16,
    pub unix_s: Option<i64>,
    pub weight: f64,
}

impl EventRow {
    pub(super) fn into_event(self) -> Option<(String, super::history::TasteEvent)> {
        let event = event_of(self.track_id, self.event_code, self.unix_s, self.weight)?;
        Some((self.user_id, event))
    }
}

pub(super) struct TasteDataset {
    path: TempPath,
    pub users: usize,
    pub timed_users: usize,
    pub items: usize,
    pub bytes: usize,
}

impl TasteDataset {
    pub(super) async fn open(&self) -> std::io::Result<tokio::fs::File> {
        tokio::fs::File::open(&self.path).await
    }
}

pub(super) enum Export {
    Ready(TasteDataset),
    TooFewUsers { users: usize, timed_users: usize },
    TooLarge { limit: usize },
}

#[derive(Serialize)]
struct Header {
    version: u32,
    since: i64,
    until: i64,
    event_types: BTreeMap<&'static str, u8>,
    users: usize,
    items: usize,
}

#[derive(Serialize)]
struct UserLine<'a> {
    u: &'a str,
    e: Vec<(u64, u8, Option<i64>, f64)>,
}

#[derive(Serialize)]
struct FeatureLine {
    i: u64,
    clap: String,
    mert: String,
    collab: Option<String>,
}

struct Section {
    path: TempPath,
    writer: BufWriter<tokio::fs::File>,
    bytes: usize,
}

impl Section {
    async fn create() -> JobResult<Self> {
        let path = tempfile::NamedTempFile::new()
            .map_err(JobError::retryable)?
            .into_temp_path();
        let file = tokio::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&path)
            .await
            .map_err(JobError::retryable)?;
        Ok(Self {
            path,
            writer: BufWriter::new(file),
            bytes: 0,
        })
    }

    async fn write_line(&mut self, line: &[u8]) -> JobResult {
        self.writer
            .write_all(line)
            .await
            .map_err(JobError::retryable)?;
        self.writer
            .write_all(b"\n")
            .await
            .map_err(JobError::retryable)?;
        self.bytes += line.len() + 1;
        Ok(())
    }

    async fn close(mut self) -> JobResult<TempPath> {
        self.writer.flush().await.map_err(JobError::retryable)?;
        Ok(self.path)
    }
}

struct UserSection {
    section: Section,
    users: usize,
    timed_users: usize,
}

pub(super) async fn export(
    pool: &PgPool,
    qdrant: &QdrantProvisioner,
    history_days: u32,
    min_users: usize,
    max_bytes: usize,
) -> JobResult<Export> {
    let until = Utc::now().timestamp();
    let Some(users) = write_users(pool, history_days, max_bytes).await? else {
        return Ok(Export::TooLarge { limit: max_bytes });
    };
    if users.timed_users < min_users {
        return Ok(Export::TooFewUsers {
            users: users.users,
            timed_users: users.timed_users,
        });
    }
    let budget = max_bytes.saturating_sub(users.section.bytes);
    let Some((items, item_count)) = write_items(qdrant, budget).await? else {
        return Ok(Export::TooLarge { limit: max_bytes });
    };
    let header = Header {
        version: TASTE_DATASET_VERSION,
        since: until - i64::from(history_days) * SECONDS_PER_DAY,
        until,
        event_types: EventKind::ALL
            .into_iter()
            .map(|kind| (kind.name(), kind.code()))
            .collect(),
        users: users.users,
        items: item_count,
    };
    let header = serde_json::to_vec(&header).map_err(JobError::permanent)?;
    let bytes = header.len() + 1 + users.section.bytes + items.bytes;
    if bytes > max_bytes {
        return Ok(Export::TooLarge { limit: max_bytes });
    }
    let (user_count, timed_users) = (users.users, users.timed_users);
    let users_path = users.section.close().await?;
    let items_path = items.close().await?;
    let mut dataset = Section::create().await?;
    dataset.write_line(&header).await?;
    for part in [&users_path, &items_path] {
        let mut source = tokio::fs::File::open(part)
            .await
            .map_err(JobError::retryable)?;
        tokio::io::copy(&mut source, &mut dataset.writer)
            .await
            .map_err(JobError::retryable)?;
    }
    Ok(Export::Ready(TasteDataset {
        path: dataset.close().await?,
        users: user_count,
        timed_users,
        items: item_count,
        bytes,
    }))
}

async fn write_users(
    pool: &PgPool,
    history_days: u32,
    max_bytes: usize,
) -> JobResult<Option<UserSection>> {
    let key = pseudonym_key();
    let days = i32::try_from(history_days).map_err(JobError::permanent)?;
    let mut written = UserSection {
        section: Section::create().await?,
        users: 0,
        timed_users: 0,
    };
    let mut grouper = HistoryGrouper::default();
    let mut rows =
        sqlx::query_file_as!(EventRow, "queries/taste/export_events.sql", days).fetch(pool);
    while let Some(row) = rows.try_next().await.map_err(JobError::retryable)? {
        let Some((user_id, event)) = row.into_event() else {
            continue;
        };
        if let Some(history) = grouper.push(&user_id, event) {
            write_user(&mut written, &key, &history).await?;
            if written.section.bytes > max_bytes {
                return Ok(None);
            }
        }
    }
    drop(rows);
    if let Some(history) = grouper.finish() {
        write_user(&mut written, &key, &history).await?;
    }
    Ok((written.section.bytes <= max_bytes).then_some(written))
}

async fn write_user(written: &mut UserSection, key: &[u8], history: &UserHistory) -> JobResult {
    let positives = history.positives();
    if positives.total == 0 {
        return Ok(());
    }
    let pseudonym = pseudonym(key, &history.user_id);
    let line = UserLine {
        u: &pseudonym,
        e: history
            .events
            .iter()
            .map(|event| (event.track, event.kind.code(), event.unix_s, event.weight))
            .collect(),
    };
    let line = serde_json::to_vec(&line).map_err(JobError::permanent)?;
    written.section.write_line(&line).await?;
    written.users += 1;
    if positives.timed >= TESTABLE_TIMED_POSITIVES {
        written.timed_users += 1;
    }
    Ok(())
}

async fn write_items(
    qdrant: &QdrantProvisioner,
    budget: usize,
) -> JobResult<Option<(Section, usize)>> {
    let mut section = Section::create().await?;
    let mut count = 0usize;
    let mut after = None;
    loop {
        let (page, next) = qdrant
            .scroll_vectors(TRACKS_CLAP, after, FEATURE_PAGE)
            .await
            .map_err(JobError::retryable)?;
        let ids: Vec<u64> = page.iter().map(|(id, _)| *id).collect();
        let (mert, collab) = tokio::join!(
            qdrant.retrieve_vectors(TRACKS_MERT, &ids),
            qdrant.retrieve_vectors(TRACKS_COLLAB, &ids)
        );
        let mert = mert.map_err(JobError::retryable)?;
        let collab = collab.map_err(JobError::retryable)?;
        for (id, clap) in page {
            let key = id.to_string();
            let Some(mert) = mert.get(&key) else {
                continue;
            };
            if !has_dimensions(&clap, TRACKS_CLAP_DIMENSIONS)
                || !has_dimensions(mert, TRACKS_MERT_DIMENSIONS)
            {
                continue;
            }
            let line = FeatureLine {
                i: id,
                clap: fp16_base64(&clap),
                mert: fp16_base64(mert),
                collab: collab
                    .get(&key)
                    .filter(|vector| has_dimensions(vector, TRACKS_COLLAB_DIMENSIONS))
                    .map(|vector| fp16_base64(vector)),
            };
            let line = serde_json::to_vec(&line).map_err(JobError::permanent)?;
            section.write_line(&line).await?;
            count += 1;
            if section.bytes > budget {
                return Ok(None);
            }
        }
        match next {
            Some(next) => after = Some(next),
            None => return Ok(Some((section, count))),
        }
    }
}

fn has_dimensions(vector: &[f32], dimensions: u64) -> bool {
    u64::try_from(vector.len()).ok() == Some(dimensions)
}

fn pseudonym_key() -> Vec<u8> {
    Uuid::new_v4()
        .as_bytes()
        .iter()
        .chain(Uuid::new_v4().as_bytes())
        .copied()
        .collect()
}

pub(super) fn pseudonym(key: &[u8], user_id: &str) -> String {
    hmac_sha256(key, user_id.as_bytes())
        .iter()
        .take(PSEUDONYM_BYTES)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let hashed_key;
    let key = if key.len() > SHA256_BLOCK {
        hashed_key = Sha256::digest(key);
        hashed_key.as_slice()
    } else {
        key
    };
    let mut inner_pad = [0x36u8; SHA256_BLOCK];
    let mut outer_pad = [0x5cu8; SHA256_BLOCK];
    for ((inner, outer), byte) in inner_pad.iter_mut().zip(outer_pad.iter_mut()).zip(key) {
        *inner ^= byte;
        *outer ^= byte;
    }
    let inner = Sha256::new()
        .chain_update(inner_pad)
        .chain_update(message)
        .finalize();
    Sha256::new()
        .chain_update(outer_pad)
        .chain_update(inner)
        .finalize()
        .into()
}

pub(super) fn fp16_base64(vector: &[f32]) -> String {
    let bytes: Vec<u8> = vector
        .iter()
        .flat_map(|value| f16_bits(*value).to_le_bytes())
        .collect();
    BASE64.encode(bytes)
}

fn f16_bits(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32;
    let mantissa = bits & 0x007f_ffff;
    if exponent == 0xff {
        let quiet = if mantissa == 0 { 0 } else { 0x0200 };
        return sign | 0x7c00 | quiet;
    }
    let half_exponent = exponent - 127 + 15;
    if half_exponent >= 0x1f {
        return sign | 0x7c00;
    }
    if half_exponent <= 0 {
        if half_exponent < -10 {
            return sign;
        }
        let full = mantissa | 0x0080_0000;
        let shift = (14 - half_exponent) as u32;
        return sign | round_half_even(full, shift) as u16;
    }
    let rounded = round_half_even(mantissa, 13);
    sign | (((half_exponent as u32) << 10) + rounded) as u16
}

fn round_half_even(value: u32, shift: u32) -> u32 {
    let kept = value >> shift;
    let dropped = value & ((1 << shift) - 1);
    let halfway = 1 << (shift - 1);
    if dropped > halfway || (dropped == halfway && kept & 1 == 1) {
        kept + 1
    } else {
        kept
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn users_are_named_by_a_keyed_hash_that_matches_the_standard() {
        let digest = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();

        assert_eq!(
            hex,
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        assert_eq!(pseudonym(b"Jefe", "what do ya want for nothing?").len(), 32);
        assert_ne!(pseudonym(b"one key", "42"), pseudonym(b"another key", "42"));
    }

    #[test]
    fn a_long_key_is_hashed_before_it_signs() {
        let key = [0xaau8; 131];
        let digest = hmac_sha256(
            &key,
            b"Test Using Larger Than Block-Size Key - Hash Key First",
        );
        let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();

        assert_eq!(
            hex,
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn half_precision_matches_numpy_rounding() {
        let cases = [
            (0.0f32, 0x0000u16),
            (-0.0, 0x8000),
            (1.0, 0x3c00),
            (-2.0, 0xc000),
            (0.5, 0x3800),
            (65504.0, 0x7bff),
            (65520.0, 0x7c00),
            (1.0e-8, 0x0000),
            (2f32.powi(-24), 0x0001),
            (2f32.powi(-25), 0x0000),
            (3.0 * 2f32.powi(-25), 0x0002),
            (2f32.powi(-14), 0x0400),
            (1.0 / 3.0, 0x3555),
            (1.0 + 2f32.powi(-11), 0x3c00),
            (1.0 + 3.0 * 2f32.powi(-11), 0x3c02),
            (f32::INFINITY, 0x7c00),
            (f32::NEG_INFINITY, 0xfc00),
        ];
        for (value, expected) in cases {
            assert_eq!(f16_bits(value), expected, "{value}");
        }
    }

    #[test]
    fn features_travel_as_little_endian_half_floats() {
        assert_eq!(
            fp16_base64(&[1.0, -2.0]),
            BASE64.encode([0x00, 0x3c, 0x00, 0xc0])
        );
    }
}
