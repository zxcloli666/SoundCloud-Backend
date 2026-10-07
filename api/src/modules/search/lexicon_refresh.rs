use sqlx::{Connection, PgPool};

const CHUNK_ROWS: i64 = 50_000;

pub(crate) async fn refresh(pool: &PgPool) -> anyhow::Result<()> {
    let mut connection = pool.acquire().await?;
    connection.close_on_drop();
    let acquired = sqlx::query_file_scalar!("../jobs/queries/search/lock_terms.sql")
        .fetch_one(&mut *connection)
        .await?;
    anyhow::ensure!(acquired, "another search terms refresh holds the lock");

    let mut transaction = connection.begin().await?;
    let changed = sqlx::query_file!("../jobs/queries/search/build_terms_delta.sql", CHUNK_ROWS)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
    sqlx::query(include_str!(
        "../../../../jobs/queries/search/index_terms_delta.sql"
    ))
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;

    for chunk in 0..changed.div_ceil(CHUNK_ROWS.unsigned_abs()) {
        sqlx::query(include_str!(
            "../../../../jobs/queries/search/apply_terms_chunk.sql"
        ))
        .bind(chunk as i64)
        .execute(&mut *connection)
        .await?;
    }
    Ok(())
}
