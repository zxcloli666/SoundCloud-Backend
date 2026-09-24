use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImpressionBatch {
    pub request_id: Uuid,
    pub impressions: Vec<Impression>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Impression {
    pub impression_id: Uuid,
    pub user_id: String,
    pub track_id: String,
    pub cluster_id: String,
    pub position: i16,
    pub score: Option<f32>,
    pub features: Option<Vec<f32>>,
    pub source: String,
    pub shown_at_unix_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HardNegative {
    pub event_id: Uuid,
    pub user_id: String,
    pub track_id: String,
    pub position_pct: f32,
    pub created_at_unix_ms: i64,
}
