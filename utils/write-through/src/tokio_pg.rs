use tokio_postgres::error::SqlState;
use tokio_postgres::types::{Json, Type};
use tokio_postgres::{Error, GenericClient};

use crate::{Kind, RACE_HINT, Raced};

const CALL: &str = include_str!("../queries/call.sql");

pub async fn call<K: Kind>(client: &impl GenericClient, args: &K) -> Result<K::Output, Error> {
    let row = client
        .query_typed_one(CALL, &[(&K::NAME, Type::TEXT), (&Json(args), Type::JSONB)])
        .await?;
    let Json(output) = row.try_get(0)?;
    Ok(output)
}

impl Raced for Error {
    fn raced(&self) -> bool {
        self.as_db_error().is_some_and(|error| {
            *error.code() == SqlState::UNIQUE_VIOLATION && error.hint() == Some(RACE_HINT)
        })
    }
}
