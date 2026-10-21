use sqlx::PgExecutor;
use uuid::Uuid;

use crate::modules::chat::model::DeviceRow;

pub struct SessionBind {
    pub chat_bind: Option<Vec<u8>>,
    pub soundcloud_user_id: Option<String>,
}

pub struct NewDevice<'a> {
    pub id: Uuid,
    pub user_id: &'a str,
    pub session_id: Uuid,
    pub auth_pub: &'a [u8],
    pub box_pub: &'a [u8],
    pub platform: &'a str,
    pub name: &'a str,
    pub app_version: &'a str,
    pub caps: &'a [String],
    pub tier: &'a str,
}

pub struct SessionMove<'a> {
    pub id: Uuid,
    pub session_id: Uuid,
    pub platform: &'a str,
    pub name: &'a str,
    pub app_version: &'a str,
    pub caps: &'a [String],
}

pub async fn session_bind(
    db: impl PgExecutor<'_>,
    session_id: Uuid,
) -> sqlx::Result<Option<SessionBind>> {
    sqlx::query_file_as!(
        SessionBind,
        "queries/chat/devices/session_bind.sql",
        session_id
    )
    .fetch_optional(db)
    .await
}

pub async fn by_id(db: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<Option<DeviceRow>> {
    sqlx::query_file_as!(DeviceRow, "queries/chat/devices/by_id.sql", id)
        .fetch_optional(db)
        .await
}

pub async fn lock_by_id(db: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<Option<DeviceRow>> {
    sqlx::query_file_as!(DeviceRow, "queries/chat/devices/lock_by_id.sql", id)
        .fetch_optional(db)
        .await
}

pub async fn live_by_user(db: impl PgExecutor<'_>, user_id: &str) -> sqlx::Result<Vec<DeviceRow>> {
    sqlx::query_file_as!(DeviceRow, "queries/chat/devices/live_by_user.sql", user_id)
        .fetch_all(db)
        .await
}

pub async fn count_live(db: impl PgExecutor<'_>, user_id: &str) -> sqlx::Result<i64> {
    sqlx::query_file_scalar!("queries/chat/devices/count_live.sql", user_id)
        .fetch_one(db)
        .await
}

pub async fn lock_user(db: impl PgExecutor<'_>, user_id: &str) -> sqlx::Result<()> {
    sqlx::query_file_scalar!("queries/chat/devices/lock_user.sql", user_id)
        .fetch_one(db)
        .await
        .map(|_| ())
}

pub async fn insert(
    db: impl PgExecutor<'_>,
    device: &NewDevice<'_>,
) -> sqlx::Result<Option<DeviceRow>> {
    sqlx::query_file_as!(
        DeviceRow,
        "queries/chat/devices/insert.sql",
        device.id,
        device.user_id,
        device.session_id,
        device.auth_pub,
        device.box_pub,
        device.platform,
        device.name,
        device.app_version,
        device.caps,
        device.tier
    )
    .fetch_optional(db)
    .await
}

pub async fn move_to_session(
    db: impl PgExecutor<'_>,
    change: &SessionMove<'_>,
) -> sqlx::Result<Option<DeviceRow>> {
    sqlx::query_file_as!(
        DeviceRow,
        "queries/chat/devices/move_to_session.sql",
        change.id,
        change.session_id,
        change.platform,
        change.name,
        change.app_version,
        change.caps
    )
    .fetch_optional(db)
    .await
}

pub async fn revoke(db: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<bool> {
    let revoked = sqlx::query_file_scalar!("queries/chat/devices/revoke.sql", id)
        .fetch_optional(db)
        .await?;
    Ok(revoked.is_some())
}

pub async fn revoke_session_holders(
    db: impl PgExecutor<'_>,
    session_id: Uuid,
    keep: Uuid,
) -> sqlx::Result<Vec<Uuid>> {
    sqlx::query_file_scalar!(
        "queries/chat/devices/revoke_session_holders.sql",
        session_id,
        keep
    )
    .fetch_all(db)
    .await
}

pub async fn session_held(db: impl PgExecutor<'_>, session_id: Uuid) -> sqlx::Result<bool> {
    sqlx::query_file_scalar!("queries/chat/devices/session_held.sql", session_id)
        .fetch_one(db)
        .await
}

pub async fn unlock_history(db: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<()> {
    sqlx::query_file!("queries/chat/devices/unlock_history.sql", id)
        .execute(db)
        .await
        .map(|_| ())
}

pub async fn touch(db: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<()> {
    sqlx::query_file!("queries/chat/devices/touch.sql", id)
        .execute(db)
        .await
        .map(|_| ())
}

pub async fn rekey_user(db: impl PgExecutor<'_>, user_id: &str) -> sqlx::Result<u64> {
    sqlx::query_file!("queries/chat/devices/rekey_user.sql", user_id)
        .execute(db)
        .await
        .map(|done| done.rows_affected())
}
