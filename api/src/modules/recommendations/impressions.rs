use std::time::Duration;

use backend_contracts::{IMPRESSION_SUBJECT, Impression, ImpressionBatch, Versioned};
use chrono::Utc;
use futures::future::try_join_all;
use tracing::warn;
use uuid::Uuid;

use super::clusters::ClusterResponse;
use super::service::RecommendationsService;

const MAX_IMPRESSIONS_PER_MESSAGE: usize = 64;
const MAX_ENCODED_BATCH_BYTES: usize = 512 * 1024;
const PUBLISH_TIMEOUT: Duration = Duration::from_millis(150);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImpressionSource {
    Home,
    Similar,
    Artist,
}

impl ImpressionSource {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Home => "home",
            Self::Similar => "similar",
            Self::Artist => "artist",
        }
    }
}

impl RecommendationsService {
    pub(crate) async fn record_impressions(
        &self,
        sc_user_id: &str,
        source: ImpressionSource,
        response: &ClusterResponse,
    ) {
        if sc_user_id.is_empty() {
            return;
        }

        let batches = build_batches(sc_user_id, source, response, Utc::now().timestamp_millis());
        let Some(request_id) = batches.first().map(|batch| batch.request_id) else {
            return;
        };
        let batch_count = batches.len();
        let publishes = batches.iter().enumerate().map(|(index, batch)| async move {
            let message_id = format!("{request_id}:{index}");
            let payload = Versioned::V1(batch);
            self.nats
                .publish_dedup(IMPRESSION_SUBJECT, &payload, &message_id)
                .await
        });

        match tokio::time::timeout(PUBLISH_TIMEOUT, try_join_all(publishes)).await {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                warn!(%error, %request_id, batch_count, "impression telemetry publish failed");
            }
            Err(_) => {
                warn!(%request_id, batch_count, "impression telemetry publish timed out");
            }
        }
    }
}

fn build_batches(
    sc_user_id: &str,
    source: ImpressionSource,
    response: &ClusterResponse,
    shown_at_unix_ms: i64,
) -> Vec<ImpressionBatch> {
    let request_id = Uuid::now_v7();
    let mut batches = Vec::new();
    let mut impressions = Vec::with_capacity(MAX_IMPRESSIONS_PER_MESSAGE);
    for cluster in &response.clusters {
        for (position, track_id) in cluster.track_ids.iter().enumerate() {
            let Ok(position) = i16::try_from(position) else {
                continue;
            };
            let observation = response.observation(track_id);
            impressions.push(Impression {
                impression_id: Uuid::now_v7(),
                user_id: sc_user_id.to_owned(),
                track_id: track_id.clone(),
                cluster_id: cluster.id.to_owned(),
                position,
                score: observation.and_then(|item| item.score),
                features: observation.and_then(|item| item.features.clone()),
                source: source.as_str().to_owned(),
                shown_at_unix_ms,
            });
            if impressions.len() == MAX_IMPRESSIONS_PER_MESSAGE {
                append_batches(request_id, std::mem::take(&mut impressions), &mut batches);
            }
        }
    }

    if !impressions.is_empty() {
        append_batches(request_id, impressions, &mut batches);
    }
    batches
}

fn append_batches(
    request_id: Uuid,
    impressions: Vec<Impression>,
    batches: &mut Vec<ImpressionBatch>,
) {
    let batch = ImpressionBatch {
        request_id,
        impressions,
    };
    if serde_json::to_vec(&Versioned::V1(&batch))
        .is_ok_and(|payload| payload.len() <= MAX_ENCODED_BATCH_BYTES)
    {
        batches.push(batch);
        return;
    }
    if batch.impressions.len() <= 1 {
        return;
    }

    let mut left = batch.impressions;
    let right = left.split_off(left.len() / 2);
    append_batches(request_id, left, batches);
    append_batches(request_id, right, batches);
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::modules::recommendations::clusters::ClusterBuilder;
    use crate::modules::recommendations::service::RecommendResult;

    #[test]
    fn batch_keeps_unknown_score_empty() {
        let mut builder = ClusterBuilder::new();
        builder.push("wave", vec!["track".to_owned()]);
        let batches = build_batches("user", ImpressionSource::Home, &builder.finish(), 100);

        assert!(matches!(
            batches.as_slice(),
            [ImpressionBatch { impressions, .. }]
                if matches!(impressions.as_slice(), [Impression { score: None, features: None, .. }])
        ));
    }

    #[test]
    fn batch_keeps_observed_score_and_features() {
        let result = RecommendResult {
            id: json!("track"),
            score: Some(0.75),
            payload: None,
            artist: None,
            genre: None,
            playback_count: None,
            features: Some(vec![0.25, 0.5]),
        };
        let mut builder = ClusterBuilder::new();
        builder.push_observed("wave", vec!["track".to_owned()], &[result]);
        let batches = build_batches("user", ImpressionSource::Home, &builder.finish(), 100);

        assert!(matches!(
            batches.as_slice(),
            [ImpressionBatch { impressions, .. }]
                if matches!(
                    impressions.as_slice(),
                    [Impression { score: Some(0.75), features: Some(features), .. }]
                        if features == &[0.25, 0.5]
                )
        ));
    }

    #[test]
    fn large_responses_are_split_with_stable_request_id() {
        let mut builder = ClusterBuilder::new();
        builder.push(
            "wave",
            (0..MAX_IMPRESSIONS_PER_MESSAGE + 1)
                .map(|index| index.to_string())
                .collect(),
        );

        let batches = build_batches("user", ImpressionSource::Home, &builder.finish(), 100);

        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].request_id, batches[1].request_id);
        assert_eq!(batches[0].impressions.len(), MAX_IMPRESSIONS_PER_MESSAGE);
        assert_eq!(batches[1].impressions.len(), 1);
    }

    #[test]
    fn encoded_batches_stay_below_the_transport_limit() {
        let mut builder = ClusterBuilder::new();
        builder.push(
            "wave",
            (0..MAX_IMPRESSIONS_PER_MESSAGE)
                .map(|index| format!("{index}-{}", "x".repeat(12_000)))
                .collect(),
        );

        let batches = build_batches("user", ImpressionSource::Home, &builder.finish(), 100);

        assert!(batches.len() > 1);
        assert!(batches.iter().all(|batch| {
            serde_json::to_vec(&Versioned::V1(batch))
                .is_ok_and(|payload| payload.len() <= MAX_ENCODED_BATCH_BYTES)
        }));
    }
}
