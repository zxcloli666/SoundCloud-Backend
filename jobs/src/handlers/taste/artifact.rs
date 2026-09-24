use std::collections::{HashMap, HashSet};

use anyhow::{Context, ensure};
use backend_contracts::vector_store::TRACKS_TASTE_DIMENSIONS;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::pooling::Pooling;

#[derive(Deserialize)]
struct ArtifactFile {
    version: String,
    trained_at: TrainedAt,
    dim: u64,
    pooling: serde_json::Value,
    #[serde(default)]
    metrics: serde_json::Value,
    items: Vec<ArtifactItem>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum TrainedAt {
    Unix(i64),
    Text(String),
}

#[derive(Deserialize)]
struct ArtifactItem {
    id: u64,
    #[serde(rename = "vec")]
    vector: Vec<f32>,
}

pub(super) struct TasteArtifact {
    pub trained_at: DateTime<Utc>,
    pub pooling_json: serde_json::Value,
    pub pooling: Pooling,
    pub metrics: serde_json::Value,
    pub items: HashMap<u64, Vec<f32>>,
}

impl TasteArtifact {
    pub(super) fn item_points(&self) -> Vec<(u64, Vec<f32>)> {
        self.items
            .iter()
            .map(|(id, vector)| (*id, vector.clone()))
            .collect()
    }
}

pub(super) fn parse(
    bytes: &[u8],
    version: &str,
    announced_items: u64,
) -> anyhow::Result<TasteArtifact> {
    let file: ArtifactFile =
        serde_json::from_slice(bytes).context("taste artifact is not valid JSON")?;
    ensure!(
        file.version == version,
        "taste artifact holds version {}, the result announced {version}",
        file.version
    );
    ensure!(
        file.dim == TRACKS_TASTE_DIMENSIONS,
        "taste artifact has {} dimensions, the contract requires {TRACKS_TASTE_DIMENSIONS}",
        file.dim
    );
    ensure!(!file.items.is_empty(), "taste artifact has no items");
    ensure!(
        u64::try_from(file.items.len()).ok() == Some(announced_items),
        "taste artifact holds {} items, the result announced {announced_items}",
        file.items.len()
    );
    let dimensions = usize::try_from(file.dim).context("taste dimensions do not fit")?;
    let mut seen = HashSet::with_capacity(file.items.len());
    let mut items = HashMap::with_capacity(file.items.len());
    for item in file.items {
        ensure!(item.id > 0, "taste artifact contains a zero track id");
        ensure!(
            seen.insert(item.id),
            "taste artifact repeats track {}",
            item.id
        );
        ensure!(
            item.vector.len() == dimensions,
            "taste vector of track {} has {} values, expected {dimensions}",
            item.id,
            item.vector.len()
        );
        ensure!(
            item.vector.iter().all(|value| value.is_finite()),
            "taste vector of track {} contains a non-finite value",
            item.id
        );
        items.insert(item.id, item.vector);
    }
    Ok(TasteArtifact {
        trained_at: trained_at(file.trained_at)?,
        pooling: Pooling::from_json(&file.pooling)?,
        pooling_json: file.pooling,
        metrics: file.metrics,
        items,
    })
}

fn trained_at(value: TrainedAt) -> anyhow::Result<DateTime<Utc>> {
    match value {
        TrainedAt::Unix(seconds) => {
            DateTime::from_timestamp(seconds, 0).context("taste trained_at is out of range")
        }
        TrainedAt::Text(text) => Ok(DateTime::parse_from_rfc3339(&text)
            .with_context(|| format!("taste trained_at {text} is not RFC 3339"))?
            .with_timezone(&Utc)),
    }
}

pub(super) const TOWER_SUFFIX: &str = "-tower";

pub(super) fn tower_object(version: &str) -> String {
    format!("{version}{TOWER_SUFFIX}")
}

pub(super) fn is_version_name(version: &str) -> bool {
    let Some(rest) = version.strip_prefix("taste-") else {
        return false;
    };
    let Some((stamp, digest)) = rest.split_once('-') else {
        return false;
    };
    stamp.len() == 12
        && stamp.bytes().all(|byte| byte.is_ascii_digit())
        && digest.len() == 8
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VERSION: &str = "taste-202609241200-0a1b2c3d";

    fn artifact(items: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "version": VERSION,
            "trained_at": "2026-09-24T12:00:00Z",
            "dim": 128,
            "pooling": {"w": {"like": 1.0}, "tau_days": 30.0},
            "metrics": {"recall_at_50": 0.3},
            "items": items
        }))
        .expect("json")
    }

    #[test]
    fn a_well_formed_artifact_is_read_whole() {
        let bytes = artifact(serde_json::json!([
            {"id": 1, "vec": vec![0.1f32; 128]},
            {"id": 2, "vec": vec![0.2f32; 128]}
        ]));

        let parsed = parse(&bytes, VERSION, 2).expect("artifact");

        assert_eq!(parsed.items.len(), 2);
        assert_eq!(parsed.trained_at.timestamp(), 1_790_251_200);
        assert_eq!(parsed.metrics["recall_at_50"], 0.3);
    }

    #[test]
    fn an_artifact_that_disagrees_with_its_result_is_refused() {
        let good = serde_json::json!([{"id": 1, "vec": vec![0.1f32; 128]}]);
        let short = serde_json::json!([{"id": 1, "vec": vec![0.1f32; 127]}]);
        let repeated = serde_json::json!([
            {"id": 1, "vec": vec![0.1f32; 128]},
            {"id": 1, "vec": vec![0.1f32; 128]}
        ]);

        assert!(parse(&artifact(good.clone()), VERSION, 2).is_err());
        assert!(parse(&artifact(good), "taste-202609241200-ffffffff", 1).is_err());
        assert!(parse(&artifact(short), VERSION, 1).is_err());
        assert!(parse(&artifact(repeated), VERSION, 2).is_err());
        assert!(parse(&artifact(serde_json::json!([])), VERSION, 0).is_err());
    }

    #[test]
    fn the_item_tower_is_the_object_the_worker_names_in_its_artifact() {
        let written_by_the_worker = serde_json::json!({
            "tower": {"object": format!("{VERSION}-tower"), "format": "safetensors"}
        });

        assert_eq!(
            written_by_the_worker["tower"]["object"],
            tower_object(VERSION).as_str()
        );
    }

    #[test]
    fn versions_follow_the_contract_pattern() {
        assert!(is_version_name(VERSION));
        assert!(!is_version_name("taste-20260924120-0a1b2c3d"));
        assert!(!is_version_name("taste-202609241200-0A1B2C3D"));
        assert!(!is_version_name("taste-202609241200"));
        assert!(!is_version_name("collab-202609241200-0a1b2c3d"));
    }
}
