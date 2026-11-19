use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{bail, ensure};
use backend_contracts::pipeline::{VideoClip, VideoPart};
use base64::Engine as _;
use sha2::{Digest, Sha256};

const STALE: Duration = Duration::from_secs(1_800);
const MAX_OPEN: usize = 6;

type Key = (String, i64, String);

struct Assembly {
    parts: BTreeMap<u32, Vec<u8>>,
    started: Instant,
}

#[derive(Default)]
pub(crate) struct Parts {
    open: Mutex<HashMap<Key, Assembly>>,
}

impl Parts {
    pub(crate) fn add(
        &self,
        track: &str,
        attempt: i64,
        clip: &VideoClip,
        part: &VideoPart,
    ) -> anyhow::Result<Option<Vec<u8>>> {
        ensure!(
            part.index < clip.parts,
            "video part {} is outside 0..{}",
            part.index,
            clip.parts
        );
        let data = base64::engine::general_purpose::STANDARD.decode(&part.data)?;
        let key = (track.to_owned(), attempt, clip.sha256.clone());
        let Ok(mut open) = self.open.lock() else {
            bail!("video parts are unavailable");
        };
        open.retain(|_, assembly| assembly.started.elapsed() < STALE);
        if !open.contains_key(&key) && open.len() >= MAX_OPEN {
            let oldest = open
                .iter()
                .min_by_key(|(_, assembly)| assembly.started)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                open.remove(&oldest);
            }
        }
        let assembly = open.entry(key.clone()).or_insert_with(|| Assembly {
            parts: BTreeMap::new(),
            started: Instant::now(),
        });
        assembly.parts.insert(part.index, data);
        if assembly.parts.len() < usize::try_from(clip.parts)? {
            return Ok(None);
        }
        let Some(done) = open.remove(&key) else {
            return Ok(None);
        };
        let file: Vec<u8> = done.parts.into_values().flatten().collect();
        ensure!(
            file.len() as u64 == clip.bytes,
            "video clip has {} bytes, the worker said {}",
            file.len(),
            clip.bytes
        );
        let digest = Sha256::digest(&file);
        let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        ensure!(hex == clip.sha256, "video clip checksum does not match");
        Ok(Some(file))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip_of(file: &[u8], parts: u32) -> VideoClip {
        let digest = Sha256::digest(file);
        VideoClip {
            bytes: file.len() as u64,
            sha256: digest.iter().map(|byte| format!("{byte:02x}")).collect(),
            parts,
        }
    }

    fn part(index: u32, data: &[u8]) -> VideoPart {
        VideoPart {
            index,
            data: base64::engine::general_purpose::STANDARD.encode(data),
        }
    }

    #[test]
    fn parts_in_any_order_rebuild_the_file_once_the_last_one_arrives() {
        let parts = Parts::default();
        let clip = clip_of(b"abcdefghi", 3);

        assert!(
            parts
                .add("1", 1, &clip, &part(2, b"ghi"))
                .unwrap()
                .is_none()
        );
        assert!(
            parts
                .add("1", 1, &clip, &part(0, b"abc"))
                .unwrap()
                .is_none()
        );
        assert!(
            parts
                .add("1", 1, &clip, &part(0, b"abc"))
                .unwrap()
                .is_none()
        );
        let file = parts.add("1", 1, &clip, &part(1, b"def")).unwrap();

        assert_eq!(file.as_deref(), Some(&b"abcdefghi"[..]));
    }

    #[test]
    fn a_wrong_checksum_or_size_is_refused() {
        let parts = Parts::default();
        let mut clip = clip_of(b"abcdef", 2);
        clip.sha256 = "0".repeat(64);
        parts.add("1", 1, &clip, &part(0, b"abc")).unwrap();
        assert!(parts.add("1", 1, &clip, &part(1, b"def")).is_err());

        let short = clip_of(b"abcdefg", 2);
        parts.add("2", 1, &short, &part(0, b"abc")).unwrap();
        assert!(parts.add("2", 1, &short, &part(1, b"def")).is_err());
    }

    #[test]
    fn a_part_outside_the_declared_count_is_refused() {
        let parts = Parts::default();

        assert!(
            parts
                .add("1", 1, &clip_of(b"ab", 1), &part(1, b"ab"))
                .is_err()
        );
    }

    #[test]
    fn the_oldest_open_file_is_dropped_past_the_cap() {
        let parts = Parts::default();
        for track in 0..=MAX_OPEN {
            let clip = clip_of(b"abcd", 2);
            parts
                .add(&track.to_string(), 1, &clip, &part(0, b"ab"))
                .unwrap();
        }

        let open = parts.open.lock().unwrap().len();

        assert_eq!(open, MAX_OPEN);
    }
}
