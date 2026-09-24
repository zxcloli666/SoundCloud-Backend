use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use backend_contracts::CollabTrainPayload;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::common::admin::AdminAuth;
use crate::error::AppResult;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/collab/status", get(status))
        .route("/admin/collab/train", post(train))
}

#[derive(Debug, Deserialize, Default)]
struct TrainBody {
    #[serde(default, rename = "minCount")]
    min_count: Option<u32>,
}

async fn status(_: AdminAuth, State(st): State<AppState>) -> AppResult<Json<Value>> {
    let dim = st.collab_vector.get_collab_dim().await;
    Ok(Json(json!({
        "collection_exists": dim.is_some(),
        "dim": dim,
    })))
}

async fn train(
    _: AdminAuth,
    State(st): State<AppState>,
    body: Option<Json<TrainBody>>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let job_id = st
        .collab_jobs
        .enqueue(CollabTrainPayload {
            min_count: body.min_count,
        })
        .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "accepted": true, "jobId": job_id })),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dimension_from_an_old_client_is_ignored_and_the_min_count_survives() {
        let body: TrainBody =
            serde_json::from_value(json!({ "dim": 64, "minCount": 3 })).expect("train body");

        assert_eq!(body.min_count, Some(3));
        assert_eq!(
            serde_json::to_value(CollabTrainPayload {
                min_count: body.min_count,
            })
            .expect("payload"),
            json!({ "min_count": 3 })
        );
    }
}
