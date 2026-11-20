use serde_json::{Value, json};

use crate::objects::Blobs;

use super::SyncQueueHandler;
use super::actions::{self, ActionError, Remote};
use super::model::ClaimedMutation;
use super::write_gate::{GatedClient, WriteGate};

pub(super) const ACTION_TYPE: &str = "playlist_artwork";
const FIELD: &str = "playlist[artwork_data]";

impl SyncQueueHandler {
    pub(super) async fn push_artwork(
        &self,
        mutation: &ClaimedMutation,
        access_token: &str,
        gate: &WriteGate<'_>,
    ) -> Result<Remote, ActionError> {
        let jpeg = self.staged_artwork(mutation).await?;
        let path = format!("/playlists/{}", mutation.target_urn);
        let response = GatedClient::new(&self.client, gate)
            .put_image(&path, access_token, FIELD, &jpeg)
            .await
            .map(|playlist| json!({"artwork_url": playlist.get("artwork_url")}));
        actions::written_or_closed(mutation, response)
    }

    pub(super) async fn discard_artwork(&self, mutation: &ClaimedMutation) {
        let (Some(store), Ok(key)) = (&self.artwork, staged_key(mutation)) else {
            return;
        };
        if let Err(error) = store.delete(key).await {
            tracing::warn!(%error, target = %mutation.target_urn, "staged playlist artwork was not deleted");
        }
    }

    async fn staged_artwork(&self, mutation: &ClaimedMutation) -> Result<Vec<u8>, ActionError> {
        let store = self.artwork.as_ref().ok_or_else(|| {
            ActionError::Staging("the artwork store is not configured".to_owned())
        })?;
        store
            .get_bytes(staged_key(mutation)?)
            .await
            .map_err(|error| ActionError::Staging(error.to_string()))?
            .ok_or(ActionError::InvalidPayload("staged artwork is gone"))
    }
}

fn staged_key(mutation: &ClaimedMutation) -> Result<&str, ActionError> {
    mutation
        .payload
        .as_ref()
        .and_then(|payload| payload.get("key"))
        .and_then(Value::as_str)
        .filter(|key| !key.is_empty())
        .ok_or(ActionError::InvalidPayload("artwork key"))
}
