use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{bail, ensure};
use backend_contracts::pipeline::{VideoClip, VideoPart};
use backend_contracts::video_gen::VideoFileSlice;
use base64::Engine as _;
use sha2::{Digest, Sha256};

const STALE: Duration = Duration::from_secs(1_800);
const MAX_OPEN: usize = 8;

type Key = (String, i64, i64, String);

struct Assembly {
    parts: BTreeMap<u32, Vec<u8>>,
    started: Instant,
}

#[derive(Default)]
pub struct Parts {
    open: Mutex<HashMap<Key, Assembly>>,
}

pub struct Delivered {
    files: BTreeMap<String, Vec<u8>>,
}

impl Parts {
    pub fn add(
        &self,
        correlation: (&str, i64, i64),
        clip: &VideoClip,
        part: &VideoPart,
    ) -> anyhow::Result<Option<Vec<u8>>> {
        ensure!(
            part.index < clip.parts,
            "part {} is outside 0..{}",
            part.index,
            clip.parts
        );
        let data = base64::engine::general_purpose::STANDARD.decode(&part.data)?;
        let (track, attempt, step) = correlation;
        let key = (track.to_owned(), attempt, step, clip.sha256.clone());
        let Ok(mut open) = self.open.lock() else {
            bail!("generation parts are unavailable");
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
        let blob: Vec<u8> = done.parts.into_values().flatten().collect();
        ensure!(
            blob.len() as u64 == clip.bytes,
            "the files have {} bytes, the worker said {}",
            blob.len(),
            clip.bytes
        );
        ensure!(
            hexed(&blob) == clip.sha256,
            "the files checksum does not match"
        );
        Ok(Some(blob))
    }
}

impl Delivered {
    pub fn split(blob: &[u8], slices: &[VideoFileSlice]) -> anyhow::Result<Self> {
        let mut files = BTreeMap::new();
        for slice in slices {
            let start = usize::try_from(slice.offset)?;
            let end = start + usize::try_from(slice.bytes)?;
            ensure!(
                end <= blob.len(),
                "file {} runs past the delivered bytes",
                slice.name
            );
            files.insert(slice.name.clone(), blob[start..end].to_vec());
        }
        Ok(Self { files })
    }

    pub fn take(&mut self, name: &str) -> anyhow::Result<Vec<u8>> {
        match self.files.remove(name) {
            Some(file) => Ok(file),
            None => bail!("the worker did not deliver {name}"),
        }
    }

    pub fn stills(&mut self) -> Vec<(String, Vec<u8>)> {
        let names: Vec<String> = self
            .files
            .keys()
            .filter(|name| name.starts_with("still"))
            .cloned()
            .collect();
        names
            .into_iter()
            .filter_map(|name| self.files.remove(&name).map(|file| (name, file)))
            .collect()
    }
}

fn hexed(blob: &[u8]) -> String {
    Sha256::digest(blob)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(blob: &[u8], parts: u32) -> VideoClip {
        VideoClip {
            bytes: blob.len() as u64,
            sha256: hexed(blob),
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
    fn parts_in_any_order_join_into_the_delivered_files() {
        let parts = Parts::default();
        let blob = b"framestill";
        let clip = clip(blob, 2);

        assert!(
            parts
                .add(("1", 1, 2), &clip, &part(1, b"still"))
                .unwrap()
                .is_none()
        );
        let joined = parts
            .add(("1", 1, 2), &clip, &part(0, b"frame"))
            .unwrap()
            .unwrap();
        let mut files = Delivered::split(
            &joined,
            &[
                VideoFileSlice {
                    name: "frame.png".to_owned(),
                    offset: 0,
                    bytes: 5,
                },
                VideoFileSlice {
                    name: "still0.jpg".to_owned(),
                    offset: 5,
                    bytes: 5,
                },
            ],
        )
        .unwrap();

        assert_eq!(files.take("frame.png").unwrap(), b"frame");
        assert_eq!(
            files.stills(),
            vec![("still0.jpg".to_owned(), b"still".to_vec())]
        );
        assert!(files.take("shot.mp4").is_err());
    }

    #[test]
    fn a_wrong_checksum_is_refused() {
        let parts = Parts::default();
        let mut bad = clip(b"abc", 1);
        bad.sha256 = "00".repeat(32);

        assert!(parts.add(("1", 1, 2), &bad, &part(0, b"abc")).is_err());
    }
}
