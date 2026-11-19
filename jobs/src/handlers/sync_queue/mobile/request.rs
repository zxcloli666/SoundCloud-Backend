use sc_mobile::reactions::{self, FIRE, TrackReaction};
use sc_mobile::{MobileRequest, comments, follows};
use serde_json::Value;

use crate::handlers::sync_queue::actions::ActionError;
use crate::handlers::sync_queue::model::ClaimedMutation;

const VALUE_PREFIX: &str = "sc:interactiontypevalue:";
const COMMENT_PREFIX: &str = "soundcloud:comments:";
const MAX_SECOND: u64 = 24 * 60 * 60;

pub(in crate::handlers::sync_queue) fn build(
    mutation: &ClaimedMutation,
) -> Result<MobileRequest, ActionError> {
    let target = mutation.target_urn.as_str();
    match mutation.action_type.as_str() {
        "track_reaction_set" => Ok(reactions::add(&reaction(mutation)?)),
        "track_reaction_remove" => Ok(reactions::remove(&reaction(mutation)?)),
        "comment_delete" if target.starts_with(COMMENT_PREFIX) => Ok(comments::delete(target)),
        "comment_delete" => Err(ActionError::InvalidPayload("comment urn")),
        "follow_user" => Ok(follows::follow(target)),
        "unfollow_user" => Ok(follows::unfollow(target)),
        action => Err(ActionError::UnknownAction(action.to_owned())),
    }
}

fn reaction(mutation: &ClaimedMutation) -> Result<TrackReaction<'_>, ActionError> {
    let (track_urn, at_second) = reactions::parse_moment_urn(&mutation.target_urn)
        .filter(|(_, second)| *second <= MAX_SECOND)
        .ok_or(ActionError::InvalidPayload("reaction moment"))?;
    let value_urn = match mutation
        .payload
        .as_ref()
        .and_then(|payload| payload.get("value_urn"))
    {
        None | Some(Value::Null) => FIRE,
        Some(value) => value
            .as_str()
            .filter(|value| value.starts_with(VALUE_PREFIX))
            .ok_or(ActionError::InvalidPayload("reaction value"))?,
    };
    Ok(TrackReaction {
        track_urn,
        at_second,
        value_urn,
    })
}
