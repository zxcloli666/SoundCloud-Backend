use sqlx::PgPool;

const AUDIO_INDEX_WIRE_STATE: &str =
    include_str!("../../../../api/migrations/0088_audio_index_wire_state.sql");
const AUDIO_INDEX_ATTEMPT: &str =
    include_str!("../../../../api/migrations/0112_audio_index_attempt.sql");

pub(super) async fn install_audio_index_wire_state(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(AUDIO_INDEX_WIRE_STATE).execute(pool).await?;
    sqlx::raw_sql(AUDIO_INDEX_ATTEMPT).execute(pool).await?;
    Ok(())
}
