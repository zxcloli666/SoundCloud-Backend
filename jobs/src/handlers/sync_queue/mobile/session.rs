use chrono::{DateTime, Utc};
use redact::Secret;
use sc_mobile::MobileApp;
use sqlx::PgPool;
use uuid::Uuid;

pub(super) const RELINK_REQUIRED: &str = "relink_required";

pub(super) struct Session {
    pub(super) connection_id: Uuid,
    pub(super) oauth_app_id: Uuid,
    pub(super) account: String,
    pub(super) device_id: String,
    pub(super) access_token: Secret<String>,
    pub(super) expires_at: DateTime<Utc>,
    pub(super) status: String,
    pub(super) plan: Option<String>,
    pub(super) app: MobileApp,
}

pub(super) async fn load(pool: &PgPool, user_id: &str) -> Result<Option<Session>, sqlx::Error> {
    let row = sqlx::query_file!("queries/sync_queue/mobile/load_session.sql", user_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.and_then(|row| {
        Some(Session {
            connection_id: row.connection_id,
            oauth_app_id: row.oauth_app_id,
            account: row.account,
            device_id: row.device_id.simple().to_string(),
            access_token: Secret::new(row.access_token),
            expires_at: row.expires_at,
            status: row.status,
            plan: row.plan,
            app: MobileApp {
                client_id: row.client_id,
                client_secret: Secret::default(),
                app_id: row.app_id?,
                app_version: row.app_version?,
                user_agent: row.user_agent?,
            },
        })
    }))
}

pub(super) async fn note_plan(pool: &PgPool, account: &str, plan: &str) -> Result<(), sqlx::Error> {
    sqlx::query_file!("queries/sync_queue/mobile/note_plan.sql", account, plan)
        .execute(pool)
        .await?;
    Ok(())
}

pub(super) async fn require_relink(
    pool: &PgPool,
    account: &str,
    rejected_token: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query_file!(
        "queries/sync_queue/mobile/relink_required.sql",
        account,
        rejected_token
    )
    .execute(pool)
    .await?;
    Ok(())
}
