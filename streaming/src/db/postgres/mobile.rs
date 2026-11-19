use redact::Secret;
use sc_mobile::MobileApp;
use tokio_postgres::Row;
use uuid::Uuid;

use super::{PgError, PgPool};

const NOTE_PLAN: &str = "UPDATE mobile_credentials
                         SET plan = $2, plan_seen_at = now()
                         WHERE soundcloud_user_id = $1 AND plan IS DISTINCT FROM $2";

pub struct MobileSession {
    pub connection_id: Uuid,
    pub account: String,
    pub device_id: String,
    pub access_token: Secret<String>,
    pub fresh: bool,
    pub plan: Option<String>,
    pub app: MobileApp,
}

impl MobileSession {
    pub(super) fn from_row(row: &Row) -> Option<Self> {
        let device: Uuid = row.get::<_, Option<Uuid>>(5)?;
        Some(Self {
            account: row.get::<_, Option<String>>(1)?,
            connection_id: row.get::<_, Option<Uuid>>(2)?,
            access_token: Secret::new(row.get::<_, Option<String>>(3)?),
            fresh: row.get::<_, Option<bool>>(4).unwrap_or(false),
            device_id: device.simple().to_string(),
            plan: row.get(6),
            app: MobileApp {
                client_id: row.get::<_, Option<String>>(7)?,
                client_secret: Secret::default(),
                app_id: row.get::<_, Option<String>>(8)?,
                app_version: row.get::<_, Option<String>>(9)?,
                user_agent: row.get::<_, Option<String>>(10)?,
            },
        })
    }
}

impl PgPool {
    pub async fn note_mobile_plan(&self, account: &str, plan: &str) -> Result<bool, PgError> {
        let client = self.connection().await?;
        let changed = client.execute(NOTE_PLAN, &[&account, &plan]).await?;
        Ok(changed > 0)
    }
}

#[cfg(test)]
#[path = "mobile_tests.rs"]
mod tests;
