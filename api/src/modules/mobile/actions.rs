use serde_json::{Value, json};

use crate::error::{AppError, AppResult};
use crate::modules::sync_queue::SyncQueueService;

use super::credential::RELINK_REQUIRED;
use super::refusal::{not_linked, relink_required};
use super::service::MobileService;
use super::validate;

const REACTION_SET: &str = "track_reaction_set";
const REACTION_REMOVE: &str = "track_reaction_remove";
const COMMENT_DELETE: &str = "comment_delete";

impl MobileService {
    pub(super) async fn set_reaction(
        &self,
        queue: &SyncQueueService,
        account: &str,
        track: &str,
        second: u64,
        value: Option<&str>,
    ) -> AppResult<Value> {
        self.react(queue, REACTION_SET, account, track, second, value)
            .await
    }

    pub(super) async fn remove_reaction(
        &self,
        queue: &SyncQueueService,
        account: &str,
        track: &str,
        second: u64,
        value: Option<&str>,
    ) -> AppResult<Value> {
        self.react(queue, REACTION_REMOVE, account, track, second, value)
            .await
    }

    pub(super) async fn delete_comment(
        &self,
        queue: &SyncQueueService,
        account: &str,
        comment: &str,
    ) -> AppResult<Value> {
        let urn = validate::comment_urn(comment)?;
        self.require_active(account).await?;
        let id = urn.rsplit(':').next().unwrap_or_default();
        let authority = sqlx::query_file!("queries/mobile/comment_authority.sql", id)
            .fetch_optional(&self.pool)
            .await?;
        if let Some(authority) = authority {
            let me = author_urn(account);
            let owns_track = authority.uploader_urn.as_deref() == Some(me.as_str())
                || authority.uploader_sc_user_id.as_deref()
                    == Some(crate::common::sc_ids::extract_sc_id(account));
            if authority.author != me && !owns_track {
                return Err(AppError::forbidden(
                    "Only your own comment or a comment on your own track can be deleted",
                ));
            }
        }
        queue.enqueue(account, COMMENT_DELETE, &urn, None).await?;
        Ok(queued(COMMENT_DELETE, &urn))
    }

    pub(super) async fn require_active(&self, account: &str) -> AppResult<()> {
        let status = sqlx::query_file_scalar!("queries/mobile/status_of.sql", account)
            .fetch_optional(&self.pool)
            .await?;
        match status.as_deref() {
            None => Err(not_linked()),
            Some(RELINK_REQUIRED) => Err(relink_required()),
            Some(_) => Ok(()),
        }
    }

    async fn react(
        &self,
        queue: &SyncQueueService,
        action: &'static str,
        account: &str,
        track: &str,
        second: u64,
        value: Option<&str>,
    ) -> AppResult<Value> {
        let track = validate::track_urn(track)?;
        let second = validate::second(second)?;
        let value = validate::value(value)?;
        self.require_active(account).await?;
        let moment = sc_mobile::reactions::moment_urn(&track, second);
        let payload = value.map(|value| json!({ "value_urn": value }));
        queue
            .enqueue(account, action, &moment, payload.as_ref())
            .await?;
        Ok(queued(action, &moment))
    }
}

fn queued(action: &str, target: &str) -> Value {
    json!({ "status": "queued", "actionType": action, "targetUrn": target })
}

fn author_urn(account: &str) -> String {
    backend_contracts::CatalogEntity::User.urn(crate::common::sc_ids::extract_sc_id(account))
}
