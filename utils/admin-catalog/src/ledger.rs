use backend_contracts::admin_catalog::{AdminCatalogReply, AdminCatalogRequest};
use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::error::{Fail, encode};

pub enum Claim {
    Fresh(Value),
    Replay(AdminCatalogReply),
    Reused,
}

pub async fn claim(
    connection: &mut PgConnection,
    request: &AdminCatalogRequest,
) -> Result<Claim, Fail> {
    sqlx::query_file_scalar!("queries/ledger/lock.sql", request.request_id.to_string())
        .fetch_one(&mut *connection)
        .await?;
    let command = encode(&request.command)?;
    let stored = sqlx::query_file!("queries/ledger/find.sql", request.request_id)
        .fetch_optional(&mut *connection)
        .await?;
    match stored {
        None => Ok(Claim::Fresh(command)),
        Some(row) if row.command == command => Ok(Claim::Replay(AdminCatalogReply {
            status: 200,
            body: row.reply,
        })),
        Some(_) => Ok(Claim::Reused),
    }
}

pub async fn store(
    connection: &mut PgConnection,
    request_id: Uuid,
    command: &Value,
    reply: &Value,
) -> Result<(), sqlx::Error> {
    sqlx::query_file!("queries/ledger/prune.sql")
        .execute(&mut *connection)
        .await?;
    sqlx::query_file!("queries/ledger/store.sql", request_id, command, reply)
        .execute(&mut *connection)
        .await?;
    Ok(())
}
