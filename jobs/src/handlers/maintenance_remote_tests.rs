use sqlx::PgPool;

use super::MaintenanceHandler;

#[sqlx::test(migrations = false)]
async fn cleanup_deletes_expired_voice_grants_and_link_codes(pool: PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(include_str!(
        "../../../api/migrations/0311_remote_voice.sql"
    ))
    .execute(&pool)
    .await?;
    sqlx::raw_sql(
        "INSERT INTO remote_grants (id, sc_user_id, provider, client_id, external_id, code_hash,
                                    code_expires_at, access_hash, access_expires_at, refresh_hash,
                                    refresh_expires_at)
         VALUES
           (gen_random_uuid(), 'refresh-expired', 'alice', 'yandex', NULL, 'c1', NULL,
            'a1', now() - interval '2 days', 'r1', now() - interval '1 second'),
           (gen_random_uuid(), 'refresh-live', 'alice', 'yandex', NULL, 'c2', NULL,
            'a2', now() - interval '2 days', 'r2', now() + interval '1 day'),
           (gen_random_uuid(), 'salut-binding', 'salut', '', 'sub-1', NULL, NULL,
            NULL, NULL, NULL, NULL),
           (gen_random_uuid(), 'code-expired', 'alice', 'yandex', NULL, 'c3',
            now() - interval '1 second', NULL, NULL, NULL, NULL),
           (gen_random_uuid(), 'code-live', 'alice', 'yandex', NULL, 'c4',
            now() + interval '5 minutes', NULL, NULL, NULL, NULL);
         INSERT INTO remote_link_codes (id, sc_user_id, provider, code_hash, expires_at)
         VALUES
           (gen_random_uuid(), 'link-expired', 'alice', 'h1', now() - interval '1 second'),
           (gen_random_uuid(), 'link-live', 'salut', 'h2', now() + interval '10 minutes');",
    )
    .execute(&pool)
    .await?;

    MaintenanceHandler::new(pool.clone())
        .cleanup_remote_links()
        .await?;

    let grants: Vec<String> =
        sqlx::query_scalar("SELECT sc_user_id FROM remote_grants ORDER BY sc_user_id")
            .fetch_all(&pool)
            .await?;
    assert_eq!(grants, ["code-live", "refresh-live", "salut-binding"]);
    let codes: Vec<String> = sqlx::query_scalar("SELECT sc_user_id FROM remote_link_codes")
        .fetch_all(&pool)
        .await?;
    assert_eq!(codes, ["link-live"]);
    Ok(())
}
