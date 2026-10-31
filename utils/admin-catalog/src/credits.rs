use backend_contracts::admin_catalog::CreditRef;
use serde_json::json;
use sqlx::PgConnection;

use crate::error::{Done, Refusal};

pub async fn accept(connection: &mut PgConnection, credit: &CreditRef) -> Done {
    let applied = sqlx::query_file!(
        "queries/credits/accept.sql",
        credit.track_id,
        credit.artist_id,
        credit.role
    )
    .execute(&mut *connection)
    .await?
    .rows_affected()
        > 0;
    if !applied {
        return Err(Refusal::not_found("weak track credit not found").into());
    }
    Ok(json!({ "applied": true, "primaryCleared": false }))
}

pub async fn reject(connection: &mut PgConnection, credit: &CreditRef) -> Done {
    let applied = sqlx::query_file!(
        "queries/credits/reject.sql",
        credit.track_id,
        credit.artist_id,
        credit.role
    )
    .execute(&mut *connection)
    .await?
    .rows_affected()
        > 0;
    if !applied {
        return Err(Refusal::not_found("weak track credit not found").into());
    }
    let primary_cleared = sqlx::query_file!(
        "queries/credits/clear_rejected_primary.sql",
        credit.track_id,
        credit.artist_id
    )
    .execute(&mut *connection)
    .await?
    .rows_affected()
        > 0;
    Ok(json!({ "applied": true, "primaryCleared": primary_cleared }))
}
