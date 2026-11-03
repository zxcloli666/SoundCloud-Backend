use backend_contracts::admin_catalog::{WantedLink, WantedStatus};
use catalog_match::link_wanted_to_sc_in;
use serde_json::json;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::error::{Done, Refusal};

const ALLOWED_STATUS: [&str; 4] = ["wanted", "linked", "unresolvable", "skipped"];

pub async fn link(connection: &mut PgConnection, wanted_id: Uuid, body: &WantedLink) -> Done {
    let sc_track_id = body.sc_track_id.trim();
    if sc_track_id.is_empty() {
        return Err(Refusal::bad_request("sc_track_id is required").into());
    }
    let linked = link_wanted_to_sc_in(connection, wanted_id, sc_track_id).await?;
    let row = sqlx::query_file!("queries/wanted/status_track.sql", wanted_id)
        .fetch_optional(&mut *connection)
        .await?;
    match row {
        None => Err(Refusal::not_found("wanted track not found").into()),
        Some(_) if !linked => Err(Refusal::bad_request(
            "no tracks row matches sc_track_id; wanted track left unlinked",
        )
        .into()),
        Some(row) => Ok(json!({
            "ok": true,
            "linked": row.track_id.is_some(),
            "status": row.status,
            "track_id": row.track_id,
        })),
    }
}

pub async fn set_status(
    connection: &mut PgConnection,
    wanted_id: Uuid,
    body: &WantedStatus,
) -> Done {
    let status = body.status.trim();
    if !ALLOWED_STATUS.contains(&status) {
        return Err(Refusal::bad_request(
            "status must be one of: wanted, linked, unresolvable, skipped",
        )
        .into());
    }
    let updated = sqlx::query_file!("queries/wanted/set_status.sql", status, wanted_id)
        .execute(&mut *connection)
        .await?;
    if updated.rows_affected() == 0 {
        return Err(Refusal::not_found("wanted track not found").into());
    }
    Ok(json!({ "ok": true, "status": status }))
}
