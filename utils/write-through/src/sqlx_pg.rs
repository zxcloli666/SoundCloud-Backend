use sqlx::PgConnection;
use sqlx::postgres::PgDatabaseError;
use sqlx::types::Json;

use crate::{Kind, RACE_HINT, Raced};

pub async fn call<K: Kind>(conn: &mut PgConnection, args: &K) -> Result<K::Output, sqlx::Error> {
    let Json(output): Json<K::Output> =
        sqlx::query_file_scalar!("queries/call.sql", K::NAME, Json(args) as _)
            .fetch_one(conn)
            .await?;
    Ok(output)
}

impl Raced for sqlx::Error {
    fn raced(&self) -> bool {
        self.as_database_error()
            .and_then(|error| error.try_downcast_ref::<PgDatabaseError>())
            .is_some_and(|error| error.code() == "23505" && error.hint() == Some(RACE_HINT))
    }
}
