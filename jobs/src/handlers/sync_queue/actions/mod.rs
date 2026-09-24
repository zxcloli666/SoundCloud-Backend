mod membership;

mod playlist;

use serde_json::Value;
use sqlx::PgConnection;

use super::client::{SoundCloudClient, SoundCloudError};
use super::model::ClaimedMutation;

#[derive(Debug, thiserror::Error)]
pub enum ActionError {
    #[error("sync action database operation failed: {0}")]
    Database(#[from] sqlx::Error),

    #[error("sync action SoundCloud request failed: {0}")]
    SoundCloud(#[from] SoundCloudError),

    #[error("invalid sync action payload: {0}")]
    InvalidPayload(&'static str),

    #[error("invalid stored SoundCloud result: {0}")]
    InvalidRemoteResult(&'static str),

    #[error("unknown sync action {0:?}")]
    UnknownAction(String),
}

pub async fn execute_remote(
    client: &SoundCloudClient,
    mutation: &ClaimedMutation,
    access_token: &str,
) -> Result<Value, ActionError> {
    let target = &mutation.target_urn;
    let result = match mutation.action_type.as_str() {
        "like_track" => {
            client
                .post(&format!("/likes/tracks/{target}"), access_token, None)
                .await?
        }
        "unlike_track" => {
            client
                .delete(&format!("/likes/tracks/{target}"), access_token)
                .await?
        }
        "like_playlist" => {
            client
                .post(&format!("/likes/playlists/{target}"), access_token, None)
                .await?
        }
        "unlike_playlist" => {
            client
                .delete(&format!("/likes/playlists/{target}"), access_token)
                .await?
        }
        "follow_user" => {
            client
                .put(&format!("/me/followings/{target}"), access_token, None)
                .await?
        }
        "unfollow_user" => {
            client
                .delete(&format!("/me/followings/{target}"), access_token)
                .await?
        }
        "playlist_create" => {
            client
                .post(
                    "/playlists",
                    access_token,
                    Some(required_payload(mutation)?),
                )
                .await?
        }
        "playlist_delete" => {
            delete_remote(client, &format!("/playlists/{target}"), access_token).await?
        }
        "track_update" => {
            let update = catalog_ingest::TrackUpdate::parse(required_payload(mutation)?)
                .map_err(ActionError::InvalidPayload)?;
            client
                .put(
                    &format!("/tracks/{target}"),
                    access_token,
                    Some(update.body()),
                )
                .await?
        }
        "track_delete" => delete_remote(client, &format!("/tracks/{target}"), access_token).await?,
        "playlist_update" => {
            let update = catalog_ingest::PlaylistUpdate::parse(required_payload(mutation)?)
                .map_err(ActionError::InvalidPayload)?;
            client
                .put(
                    &format!("/playlists/{target}"),
                    access_token,
                    Some(update.body()),
                )
                .await?
        }
        membership::ACTION_TYPE => {
            let apply = membership::MembershipApply::parse(required_payload(mutation)?)
                .map_err(ActionError::InvalidPayload)?;
            if apply.is_empty() {
                tracing::warn!(
                    playlist = %target,
                    fingerprint = %apply.fingerprint,
                    reconcile_generation = apply.reconcile_generation,
                    "applying an empty membership clears the remote playlist"
                );
            } else {
                tracing::info!(
                    playlist = %target,
                    tracks = apply.len(),
                    fingerprint = %apply.fingerprint,
                    reconcile_generation = apply.reconcile_generation,
                    "applying playlist membership to SoundCloud"
                );
            }
            client
                .put(
                    &format!("/playlists/{target}"),
                    access_token,
                    Some(&apply.body()),
                )
                .await?
        }
        "comment" => {
            client
                .post(
                    &format!("/tracks/{target}/comments"),
                    access_token,
                    Some(required_payload(mutation)?),
                )
                .await?
        }
        action => return Err(ActionError::UnknownAction(action.to_owned())),
    };
    Ok(result)
}

pub async fn apply_local(
    connection: &mut PgConnection,
    mutation: &ClaimedMutation,
    remote_result: &Value,
) -> Result<(), ActionError> {
    let user_id = extract_sc_id(&mutation.user_id);
    let target = &mutation.target_urn;
    match mutation.action_type.as_str() {
        "like_track" => {
            sqlx::query_file!(
                "queries/sync_queue/actions/like_track.sql",
                user_id,
                extract_sc_id(target)
            )
            .execute(connection)
            .await?;
        }
        "unlike_track" => {
            sqlx::query_file!(
                "queries/sync_queue/actions/unlike_track.sql",
                user_id,
                extract_sc_id(target)
            )
            .execute(connection)
            .await?;
        }
        "like_playlist" => {
            sqlx::query_file!(
                "queries/sync_queue/actions/like_playlist.sql",
                user_id,
                target
            )
            .execute(connection)
            .await?;
        }
        "unlike_playlist" => {
            sqlx::query_file!(
                "queries/sync_queue/actions/unlike_playlist.sql",
                user_id,
                target
            )
            .execute(connection)
            .await?;
        }
        "follow_user" => {
            sqlx::query_file!(
                "queries/sync_queue/actions/follow_user.sql",
                user_id,
                target
            )
            .execute(connection)
            .await?;
        }
        "unfollow_user" => {
            sqlx::query_file!(
                "queries/sync_queue/actions/unfollow_user.sql",
                user_id,
                target
            )
            .execute(connection)
            .await?;
        }
        "playlist_create" => {
            playlist::finalize_create(connection, mutation.id, user_id, remote_result).await?;
        }
        "playlist_delete" => {
            let variants = user_id_variants(user_id);
            sqlx::query_file!(
                "queries/sync_queue/actions/delete_owned_playlist.sql",
                &variants,
                target
            )
            .execute(&mut *connection)
            .await?;
            sqlx::query_file!(
                "queries/sync_queue/actions/confirm_playlist_delete.sql",
                target
            )
            .execute(connection)
            .await?;
        }
        "track_update" => {
            let update = catalog_ingest::TrackUpdate::parse(required_payload(mutation)?)
                .map_err(ActionError::InvalidPayload)?;
            sqlx::query_file!(
                "queries/sync_queue/actions/confirm_track_update.sql",
                extract_sc_id(target),
                update.desired()
            )
            .execute(connection)
            .await?;
        }
        "track_delete" => {
            sqlx::query_file!(
                "queries/sync_queue/actions/confirm_track_delete.sql",
                extract_sc_id(target)
            )
            .execute(connection)
            .await?;
        }
        "playlist_update" => {
            let update = catalog_ingest::PlaylistUpdate::parse(required_payload(mutation)?)
                .map_err(ActionError::InvalidPayload)?;
            sqlx::query_file!(
                "queries/sync_queue/actions/confirm_playlist_update.sql",
                target,
                update.desired()
            )
            .execute(connection)
            .await?;
        }
        membership::ACTION_TYPE => {
            let apply = membership::MembershipApply::parse(required_payload(mutation)?)
                .map_err(ActionError::InvalidPayload)?;
            sqlx::query_file!(
                "queries/sync_queue/actions/confirm_membership_apply.sql",
                target,
                &apply.fingerprint,
                apply.reconcile_generation
            )
            .execute(connection)
            .await?;
        }
        "comment" => {}
        action => return Err(ActionError::UnknownAction(action.to_owned())),
    }
    Ok(())
}

async fn delete_remote(
    client: &SoundCloudClient,
    path: &str,
    access_token: &str,
) -> Result<Value, ActionError> {
    match client.delete(path, access_token).await {
        Ok(value) => Ok(value),
        Err(SoundCloudError::Api {
            status: wreq::StatusCode::NOT_FOUND | wreq::StatusCode::GONE,
            ..
        }) => Ok(Value::Null),
        Err(error) => Err(error.into()),
    }
}

fn required_payload(mutation: &ClaimedMutation) -> Result<&Value, ActionError> {
    mutation
        .payload
        .as_ref()
        .ok_or(ActionError::InvalidPayload("payload is missing"))
}

fn extract_sc_id(value: &str) -> &str {
    value.rsplit(':').next().unwrap_or(value)
}

fn user_id_variants(user_id: &str) -> [String; 2] {
    [user_id.to_owned(), format!("soundcloud:users:{user_id}")]
}

#[cfg(test)]
mod tests;
