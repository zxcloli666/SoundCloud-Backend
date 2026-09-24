use backend_contracts::reasons::WorkerStatus;
use qdrant_client::qdrant::SearchPointsBuilder;
use tracing::debug;

use crate::error::AppResult;
use crate::modules::lyrics::EncodeOutcome;
use crate::qdrant::collections;

use super::service::util::{payload_to_map, point_id_to_value, value_to_u64};
use super::service::{RecommendResult, RecommendationsService};

const FEATURE_LEN: usize = 8;
const CANDIDATES_PER_RESULT: usize = 3;
const MIN_CANDIDATES: usize = 40;
const MAX_CANDIDATES: usize = 500;

fn fetch_size(limit: usize) -> usize {
    limit
        .saturating_mul(CANDIDATES_PER_RESULT)
        .clamp(MIN_CANDIDATES, MAX_CANDIDATES)
}

#[derive(Debug, Default)]
pub struct SearchTextResult {
    pub preparing: bool,
    pub failed: bool,
    pub results: Vec<RecommendResult>,
}

fn query_vector(outcome: EncodeOutcome) -> Result<Vec<f32>, SearchTextResult> {
    match outcome {
        EncodeOutcome::Ready(vector) if !vector.is_empty() => Ok(vector),
        EncodeOutcome::Preparing => Err(SearchTextResult {
            preparing: true,
            ..SearchTextResult::default()
        }),
        EncodeOutcome::Ready(_)
        | EncodeOutcome::Declined {
            status: WorkerStatus::Empty,
            ..
        } => Err(SearchTextResult::default()),
        EncodeOutcome::Declined { .. } => Err(SearchTextResult {
            failed: true,
            ..SearchTextResult::default()
        }),
    }
}

impl RecommendationsService {
    pub async fn search_by_text(
        &self,
        query: &str,
        limit: usize,
        languages: Option<&[String]>,
    ) -> AppResult<SearchTextResult> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(SearchTextResult::default());
        }
        let vec = match query_vector(self.worker.encode_text_mulan(q).await?) {
            Ok(vector) => vector,
            Err(answer) => return Ok(answer),
        };
        let filter = self.build_filter(&[], languages);
        let fetch_limit = fetch_size(limit);

        let mut builder =
            SearchPointsBuilder::new(collections::TRACKS_CLAP, vec, fetch_limit as u64)
                .with_payload(true);
        if let Some(f) = filter {
            builder = builder.filter(f);
        }
        let resp = match self.qdrant.raw().search_points(builder).await {
            Ok(r) => r,
            Err(e) => {
                debug!(error = %e, "searchByText: qdrant search failed");
                return Ok(SearchTextResult {
                    failed: true,
                    ..Default::default()
                });
            }
        };

        let scored: Vec<super::service::ScoredCandidate> = resp
            .result
            .into_iter()
            .filter_map(|p| {
                let id_val = point_id_to_value(p.id);
                let id = value_to_u64(&id_val)?;
                Some(super::service::ScoredCandidate {
                    id,
                    score: p.score,
                    payload: Some(payload_to_map(p.payload)),
                    features: vec![0.0; FEATURE_LEN],
                })
            })
            .collect();

        let public = self
            .public_track_ids(&scored.iter().map(|c| c.id.to_string()).collect::<Vec<_>>())
            .await;
        let scored: Vec<super::service::ScoredCandidate> = scored
            .into_iter()
            .filter(|c| public.contains(&c.id.to_string()))
            .collect();

        let enriched = self.enrich_and_boost(scored, languages).await?;
        let diverse = self.artist_cap(enriched, self.cfg.artist_cap);
        let results = self.take_verified(diverse, limit).await?;
        Ok(SearchTextResult {
            preparing: false,
            failed: false,
            results,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use backend_contracts::reasons::WorkerReason;

    fn answer_to(outcome: EncodeOutcome) -> (bool, bool) {
        let answer = query_vector(outcome).expect_err("no vector to search with");
        assert!(answer.results.is_empty());
        (answer.preparing, answer.failed)
    }

    #[test]
    fn a_worker_failure_is_reported_as_a_failure_and_not_as_no_results() {
        assert_eq!(
            answer_to(EncodeOutcome::Declined {
                status: WorkerStatus::Failed,
                reason: Some(WorkerReason::DeadlineExceeded),
            }),
            (false, true),
            "an empty ready answer here is cached as the truth for every listener"
        );
        assert_eq!(
            answer_to(EncodeOutcome::Declined {
                status: WorkerStatus::Failed,
                reason: Some(WorkerReason::ModelOutputInvalid),
            }),
            (false, true)
        );
        assert_eq!(
            answer_to(EncodeOutcome::Declined {
                status: WorkerStatus::Empty,
                reason: Some(WorkerReason::EmptyText),
            }),
            (false, false)
        );
        assert_eq!(answer_to(EncodeOutcome::Preparing), (true, false));
        assert_eq!(
            query_vector(EncodeOutcome::Ready(vec![0.5; 512])).expect("a vector"),
            vec![0.5; 512]
        );
    }

    #[test]
    fn a_page_asks_qdrant_for_a_few_candidates_per_result() {
        assert_eq!(fetch_size(20), 60);
        assert_eq!(fetch_size(4), MIN_CANDIDATES);
    }

    #[test]
    fn no_page_size_can_turn_into_a_scan_of_the_whole_collection() {
        assert_eq!(fetch_size(1_000_000), MAX_CANDIDATES);
        assert_eq!(
            fetch_size(usize::MAX),
            MAX_CANDIDATES,
            "a page size nobody clamped must not overflow into a wrapped fetch size either"
        );
    }

    #[test]
    fn every_recommendations_handler_bounds_the_page_it_was_asked_for() {
        let handlers = std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("src/modules/recommendations/handlers.rs"),
        )
        .expect("the handlers are readable");
        let mut bounded = 0;
        for (number, line) in handlers.lines().enumerate() {
            if !line.contains("parse_limit(") || line.trim_start().starts_with("fn parse_limit") {
                continue;
            }
            bounded += 1;
            assert!(
                line.contains(".clamp("),
                "handlers.rs:{} asks for a page size without bounding it; that size is \
                 multiplied and handed to Qdrant as a fetch size, so one request can ask for \
                 the whole collection: {}",
                number + 1,
                line.trim()
            );
        }
        assert!(
            bounded >= 4,
            "only {bounded} handlers parse a page size; this guard is reading the wrong file"
        );
    }
}
