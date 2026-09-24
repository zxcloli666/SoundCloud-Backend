use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::sc_ids::extract_sc_id;
use crate::error::{AppError, AppResult};
use crate::modules::users::{UserRow, project_to_sc_shape as project_user};

const MAX_BODY_BYTES: usize = 16 * 1024;

pub struct CommentRow {
    pub id: Uuid,
    pub sc_comment_id: Option<String>,
    pub user_urn: String,
    pub body: String,
    pub track_position_ms: Option<i64>,
    pub sc_created_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

pub struct SubmittedComment {
    pub body: String,
    pub track_position_ms: Option<i64>,
}

pub fn submitted(body: &Value) -> AppResult<SubmittedComment> {
    let comment = body.get("comment").unwrap_or(body);
    let text = comment
        .get("body")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty() && text.len() <= MAX_BODY_BYTES)
        .ok_or_else(|| AppError::bad_request("comment body is missing or too long"))?;
    let position = match comment.get("timestamp") {
        None | Some(Value::Null) => None,
        Some(Value::Number(number)) => Some(
            number
                .as_i64()
                .filter(|value| *value >= 0)
                .ok_or_else(|| AppError::bad_request("comment timestamp is invalid"))?,
        ),
        Some(_) => return Err(AppError::bad_request("comment timestamp is invalid")),
    };
    Ok(SubmittedComment {
        body: text.to_owned(),
        track_position_ms: position,
    })
}

pub async fn record_pending(
    connection: &mut sqlx::PgConnection,
    sc_track_id: &str,
    sc_user_id: &str,
    comment: &SubmittedComment,
) -> AppResult<()> {
    sqlx::query_file!(
        "queries/tracks/comments/insert_pending.sql",
        Uuid::now_v7(),
        sc_track_id,
        backend_contracts::CatalogEntity::User.urn(extract_sc_id(sc_user_id)),
        &comment.body,
        comment.track_position_ms
    )
    .execute(connection)
    .await?;
    Ok(())
}

fn project(row: &CommentRow, sc_track_id: &str, author: &Value) -> Value {
    let created_at = row.sc_created_at.unwrap_or(row.created_at).to_rfc3339();
    let id = row
        .sc_comment_id
        .as_deref()
        .and_then(|id| id.parse::<i64>().ok());
    json!({
        "kind": "comment",
        "id": id,
        "urn": match row.sc_comment_id.as_deref() {
            Some(id) => format!("soundcloud:comments:{id}"),
            None => format!("local:comments:{}", row.id),
        },
        "pending": row.sc_comment_id.is_none(),
        "body": row.body,
        "created_at": created_at,
        "timestamp": row.track_position_ms,
        "track_id": sc_track_id.parse::<i64>().ok(),
        "user_id": extract_sc_id(&row.user_urn).parse::<i64>().ok(),
        "user": author,
    })
}

pub async fn read_page(
    pg: &PgPool,
    sc_track_id: &str,
    page: i64,
    limit: i64,
) -> AppResult<crate::cache::ListPageResult<Value>> {
    if !(1..=200).contains(&limit) {
        return Err(AppError::bad_request(
            "Comment page size must be between 1 and 200",
        ));
    }
    let page = page.clamp(0, 100);
    let offset = page
        .checked_mul(limit)
        .ok_or_else(|| AppError::bad_request("Comment page is out of range"))?;
    let rows: Vec<CommentRow> = sqlx::query_file_as!(
        CommentRow,
        "queries/tracks/comments/page.sql",
        sc_track_id,
        limit + 1,
        offset
    )
    .fetch_all(pg)
    .await?;
    let has_more = page < 100 && rows.len() as i64 > limit;
    let rows: Vec<CommentRow> = rows.into_iter().take(limit as usize).collect();
    let urns: Vec<String> = rows.iter().map(|row| row.user_urn.clone()).collect();
    let authors: Vec<UserRow> = sqlx::query_file_as!(
        UserRow,
        "queries/cold_refresh/service/users_by_urns.sql",
        &urns
    )
    .fetch_all(pg)
    .await?;
    let authors: std::collections::HashMap<String, UserRow> = authors
        .into_iter()
        .map(|author| (author.urn.clone(), author))
        .collect();
    let collection = rows
        .iter()
        .filter_map(|row| {
            authors
                .get(&row.user_urn)
                .map(|author| project(row, sc_track_id, &project_user(author)))
        })
        .collect();
    Ok(crate::cache::ListPageResult {
        collection,
        page,
        page_size: limit,
        has_more,
    })
}

#[cfg(test)]
#[path = "comments_tests.rs"]
mod tests;
