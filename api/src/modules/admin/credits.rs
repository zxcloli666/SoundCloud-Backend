use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::common::admin::AdminAuth;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct WeakCreditQuery {
    #[serde(default)]
    pub evidence: Option<String>,
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Deserialize)]
pub struct CreditRef {
    pub track_id: Uuid,
    pub artist_id: Uuid,
    pub role: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeakCreditRow {
    pub track_id: Uuid,
    pub artist_id: Uuid,
    pub role: String,
    pub source: String,
    pub confidence: f32,
    pub evidence: String,
    pub sc_track_id: String,
    pub track_title: String,
    pub uploader_username: Option<String>,
    pub uploader_sc_user_id: Option<String>,
    pub artist_name: String,
    pub mb_artist_id: Option<String>,
    pub genius_artist_id: Option<String>,
    pub artist_has_identity: bool,
    pub is_primary: bool,
    pub why: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceCount {
    pub evidence: String,
    pub count: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeakCreditsPage {
    pub items: Vec<WeakCreditRow>,
    pub page: i64,
    pub limit: i64,
    pub by_evidence: Vec<EvidenceCount>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditDecision {
    pub applied: bool,
    pub primary_cleared: bool,
}

fn explain(evidence: &str, has_identity: bool) -> &'static str {
    match evidence {
        "uploader_name" if has_identity => {
            "credited from the uploader name; the artist has a verified identity for another account"
        }
        "uploader_name" => "credited from the uploader name alone, with no verified identity",
        "title_heuristic" => "parsed out of the track title, with no external confirmation",
        "ai_inference" => "inferred by the AI resolver, with no external confirmation",
        _ => "carries no recorded provenance",
    }
}

#[tracing::instrument(skip_all)]
pub async fn list_weak_credits(
    _: AdminAuth,
    State(state): State<AppState>,
    Query(q): Query<WeakCreditQuery>,
) -> AppResult<Json<WeakCreditsPage>> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let page = q.page.unwrap_or(1).max(1);
    let offset = (page - 1) * limit;
    let evidence = q.evidence.filter(|value| !value.is_empty());

    let counts = sqlx::query_file!("queries/admin/catalog/weak_credit_counts.sql")
        .fetch_all(&state.pg)
        .await?;
    let rows = sqlx::query_file!(
        "queries/admin/catalog/weak_credits.sql",
        evidence.as_deref(),
        limit,
        offset
    )
    .fetch_all(&state.pg)
    .await?;

    Ok(Json(WeakCreditsPage {
        items: rows
            .into_iter()
            .map(|row| WeakCreditRow {
                why: explain(&row.evidence, row.artist_has_identity),
                track_id: row.track_id,
                artist_id: row.artist_id,
                role: row.role,
                source: row.source,
                confidence: row.confidence,
                evidence: row.evidence,
                sc_track_id: row.sc_track_id,
                track_title: row.track_title,
                uploader_username: row.uploader_username,
                uploader_sc_user_id: row.uploader_sc_user_id,
                artist_name: row.artist_name,
                mb_artist_id: row.mb_artist_id,
                genius_artist_id: row.genius_artist_id,
                artist_has_identity: row.artist_has_identity,
                is_primary: row.is_primary,
            })
            .collect(),
        page,
        limit,
        by_evidence: counts
            .into_iter()
            .map(|row| EvidenceCount {
                evidence: row.evidence,
                count: row.count,
            })
            .collect(),
    }))
}

#[tracing::instrument(skip_all)]
pub async fn accept_credit(
    _: AdminAuth,
    State(state): State<AppState>,
    Json(credit): Json<CreditRef>,
) -> AppResult<Json<CreditDecision>> {
    let applied = sqlx::query_file!(
        "queries/admin/catalog/accept_credit.sql",
        credit.track_id,
        credit.artist_id,
        credit.role
    )
    .execute(&state.pg)
    .await?
    .rows_affected()
        > 0;
    if !applied {
        return Err(AppError::not_found("weak track credit not found"));
    }
    Ok(Json(CreditDecision {
        applied,
        primary_cleared: false,
    }))
}

#[tracing::instrument(skip_all)]
pub async fn reject_credit(
    _: AdminAuth,
    State(state): State<AppState>,
    Json(credit): Json<CreditRef>,
) -> AppResult<Json<CreditDecision>> {
    let mut transaction = state.pg.begin().await?;
    let applied = sqlx::query_file!(
        "queries/admin/catalog/reject_credit.sql",
        credit.track_id,
        credit.artist_id,
        credit.role
    )
    .execute(&mut *transaction)
    .await?
    .rows_affected()
        > 0;
    if !applied {
        transaction.commit().await?;
        return Err(AppError::not_found("weak track credit not found"));
    }
    let primary_cleared = sqlx::query_file!(
        "queries/admin/catalog/clear_rejected_primary.sql",
        credit.track_id,
        credit.artist_id
    )
    .execute(&mut *transaction)
    .await?
    .rows_affected()
        > 0;
    transaction.commit().await?;

    Ok(Json(CreditDecision {
        applied,
        primary_cleared,
    }))
}
