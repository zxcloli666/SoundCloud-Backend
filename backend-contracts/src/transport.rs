use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::JobKind;

pub const JOB_INGRESS_STREAM: &str = "API_BACKGROUND_JOBS";
pub const JOB_INGRESS_SUBJECT: &str = "api.background_jobs.v1";
pub const JOB_INGRESS_CONSUMER: &str = "jobs-api-background-v1";

pub const IMPRESSION_STREAM: &str = "RECOMMENDATION_IMPRESSIONS";
pub const IMPRESSION_SUBJECT: &str = "telemetry.recommendations.impressions.v1";
pub const IMPRESSION_CONSUMER: &str = "jobs-recommendation-impressions-v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobCommand {
    pub id: Uuid,
    pub kind: JobKind,
    pub dedup_key: Option<String>,
    #[serde(default)]
    pub enqueue_if_absent: bool,
    pub payload: Value,
    pub priority: i16,
    pub max_attempts: i16,
    pub available_at_unix_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_commands_default_to_coalescing_delivery() {
        let command: JobCommand = serde_json::from_value(serde_json::json!({
            "id": Uuid::nil(),
            "kind": "discover_aggregates",
            "dedupKey": "summary",
            "payload": {},
            "priority": 0,
            "maxAttempts": 8,
            "availableAtUnixMs": 0
        }))
        .expect("legacy command deserializes");

        assert!(!command.enqueue_if_absent);
    }
}
