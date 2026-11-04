use sqlx::PgPool;
use uuid::Uuid;

use super::support::ok;
use crate::modules::chat::sweeps::reports;
use crate::modules::chat::test_support::chat_state;

async fn report(pg: &PgPool, state: &str, purge_after: Option<&str>) -> anyhow::Result<Uuid> {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO chat.reports
            (id, reporter, target_user, conversation_id, seq_from, seq_to, reason, weight,
             evidence, evidence_kek_version, state, purge_after)
         VALUES ($1, 'r', 't', gen_random_uuid(), 1, 2, 'spam', 1, '\\x03'::bytea, 1, $2,
                 now() + $3::text::interval)",
    )
    .bind(id)
    .bind(state)
    .bind(purge_after)
    .execute(pg)
    .await?;
    Ok(id)
}

async fn evidence(pg: &PgPool, id: Uuid) -> anyhow::Result<(bool, bool, bool)> {
    Ok(sqlx::query_as(
        "SELECT evidence IS NOT NULL, evidence_kek_version IS NOT NULL, purge_after IS NOT NULL
         FROM chat.reports WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pg)
    .await?)
}

#[sqlx::test(migrations = "./migrations")]
async fn evidence_goes_once_its_retention_ends(pg: PgPool) -> anyhow::Result<()> {
    let chat = chat_state(
        pg.clone(),
        crate::redis::connect_url("redis://127.0.0.1:1")?,
        true,
    );
    let due = report(&pg, "actioned", Some("-1 day")).await?;
    let waiting = report(&pg, "rejected", Some("10 days")).await?;
    let open = report(&pg, "open", None).await?;

    assert_eq!(ok(reports::run_once(&chat).await)?, 1);

    assert_eq!(evidence(&pg, due).await?, (false, false, false));
    assert_eq!(evidence(&pg, waiting).await?, (true, true, true));
    assert_eq!(evidence(&pg, open).await?, (true, true, false));
    let kept: i64 = sqlx::query_scalar("SELECT count(*) FROM chat.reports")
        .fetch_one(&pg)
        .await?;
    assert_eq!(kept, 3, "the report itself stays for the history");
    assert_eq!(ok(reports::run_once(&chat).await)?, 0);
    Ok(())
}
