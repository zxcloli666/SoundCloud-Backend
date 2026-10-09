use serde::Serialize;
use uuid::Uuid;

use crate::error::AppResult;
use crate::modules::auth::AuthService;
use crate::modules::sync_queue::SyncQueueService;

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OldAuthStatus {
    authenticated: bool,
    token_state: &'static str,
    pending_sync_count: i64,
    failed_sync_count: i64,
}

const SIGNED_OUT: OldAuthStatus = OldAuthStatus {
    authenticated: false,
    token_state: "expired",
    pending_sync_count: 0,
    failed_sync_count: 0,
};

pub(super) async fn read(
    auth: &AuthService,
    sync_queue: &SyncQueueService,
    session_id: Option<Uuid>,
) -> AppResult<OldAuthStatus> {
    let Some(session_id) = session_id else {
        return Ok(SIGNED_OUT);
    };
    let session = auth.get_auth_session(session_id).await?;
    let Some(sc_user_id) = session.and_then(|session| session.soundcloud_user_id) else {
        return Ok(SIGNED_OUT);
    };
    let counts = sync_queue.status_for_user(&sc_user_id).await?;
    Ok(OldAuthStatus {
        authenticated: true,
        token_state: "ok",
        pending_sync_count: counts.pending_count,
        failed_sync_count: counts.failed_count,
    })
}

#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;
