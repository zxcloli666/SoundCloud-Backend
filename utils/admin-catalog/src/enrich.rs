use backend_contracts::admin_catalog::{EnrichRetry, ScAccountUpsert};
use catalog_match::{AccountRole, claims_another_artist, delete_account_in, upsert_account_in};
use serde_json::json;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::error::{Done, Fail, Refusal};

pub async fn retry(connection: &mut PgConnection, body: &EnrichRetry) -> Done {
    let mut reset = 0u64;
    if let Some(raw) = body.sc_track_id.as_deref() {
        let sc_track_id = entity_ref::sc_track_id(raw)
            .ok_or_else(|| Refusal::bad_request("invalid sc_track_id"))?;
        reset += sqlx::query_file!("queries/enrich/retry_by_sc_track_id.sql", &sc_track_id)
            .execute(&mut *connection)
            .await?
            .rows_affected();
    }
    if body.all_failed {
        reset += sqlx::query_file!("queries/enrich/retry_all_failed.sql")
            .execute(&mut *connection)
            .await?
            .rows_affected();
    }
    Ok(json!({ "reset": reset }))
}

pub async fn upsert_account(
    connection: &mut PgConnection,
    artist_id: Uuid,
    body: &ScAccountUpsert,
) -> Done {
    let role = AccountRole::parse(&body.role)
        .ok_or_else(|| Refusal::bad_request("role must be main|demo|alt"))?;
    let sc_user_id = body.sc_user_id.trim();
    if sc_user_id.is_empty() {
        return Err(Refusal::bad_request("sc_user_id required").into());
    }
    upsert_account_in(connection, artist_id, sc_user_id, role, "manual", true)
        .await
        .map_err(claimed_or_database)?;
    if let Some(notes) = body.notes.as_deref() {
        sqlx::query_file!(
            "queries/artists/sc_account_notes.sql",
            artist_id,
            sc_user_id,
            notes
        )
        .execute(&mut *connection)
        .await?;
    }
    Ok(json!({ "ok": true }))
}

pub async fn delete_account(
    connection: &mut PgConnection,
    artist_id: Uuid,
    sc_user_id: &str,
) -> Done {
    let removed = delete_account_in(connection, artist_id, sc_user_id).await?;
    Ok(json!({ "ok": removed }))
}

fn claimed_or_database(error: sqlx::Error) -> Fail {
    if claims_another_artist(&error) {
        return Refusal::conflict("this SoundCloud account is already verified for another artist")
            .into();
    }
    error.into()
}
