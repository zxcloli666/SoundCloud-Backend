use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::Response;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::common::session::SessionCtx;
use crate::error::AppResult;

use super::handlers::no_store;
use super::limits::Action;
use super::refusal::unavailable;
use super::service::MobileService;
use super::validate;

const OP: &str = "reactions_read";

#[derive(Deserialize)]
pub(super) struct SecondsQuery {
    seconds: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TrackReactions {
    pub(super) track_urn: String,
    pub(super) moments: Vec<Moment>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Moment {
    pub(super) second: u64,
    pub(super) urn: String,
    pub(super) total: u64,
    pub(super) counts: Vec<ValueCount>,
    pub(super) own: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ValueCount {
    pub(super) value_urn: String,
    pub(super) count: u64,
}

#[derive(Deserialize)]
pub(super) struct Reply {
    pub(super) data: Option<Data>,
    #[serde(default)]
    pub(super) errors: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Data {
    pub(super) target_interactions: Option<Vec<Target>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Target {
    pub(super) target_urn: String,
    pub(super) user_interaction: Option<String>,
    pub(super) interaction_counts: Option<Vec<Count>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Count {
    pub(super) interaction_type_value_urn: String,
    #[serde(default)]
    pub(super) count: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReactionValue {
    value_urn: &'static str,
    emoji: &'static str,
}

#[tracing::instrument(skip_all)]
pub(super) async fn values(_ctx: SessionCtx) -> Response {
    no_store(value_list())
}

fn value_list() -> Vec<ReactionValue> {
    sc_mobile::reactions::VALUES
        .iter()
        .map(|value| ReactionValue {
            value_urn: value.value_urn,
            emoji: value.emoji,
        })
        .collect()
}

#[tracing::instrument(skip_all)]
pub(super) async fn read(
    State(mobile): State<Arc<MobileService>>,
    ctx: SessionCtx,
    Path(track_urn): Path<String>,
    Query(query): Query<SecondsQuery>,
) -> AppResult<Response> {
    mobile.limits.admit(Action::Read, ctx.session_id)?;
    let reactions = mobile
        .track_reactions(ctx.session_id, &ctx.sc_user_id, &track_urn, &query.seconds)
        .await?;
    Ok(no_store(reactions))
}

impl MobileService {
    pub(super) async fn track_reactions(
        &self,
        session_id: Uuid,
        account: &str,
        track: &str,
        seconds: &str,
    ) -> AppResult<TrackReactions> {
        let track = validate::track_urn(track)?;
        let seconds = validate::seconds(seconds)?;
        let request = sc_mobile::reactions::read(&track, &seconds);
        let reply = self.owner_reply(session_id, account, OP, &request).await?;
        let moments = moments(&reply.body, &track, &seconds).ok_or_else(unavailable)?;
        Ok(TrackReactions {
            track_urn: track,
            moments,
        })
    }
}

fn moments(body: &[u8], track: &str, seconds: &[u64]) -> Option<Vec<Moment>> {
    let reply: Reply = serde_json::from_slice(body).ok()?;
    if !reply.errors.is_empty() {
        return None;
    }
    let targets = reply.data?.target_interactions.unwrap_or_default();
    Some(
        seconds
            .iter()
            .map(|second| moment(track, *second, &targets))
            .collect(),
    )
}

fn moment(track: &str, second: u64, targets: &[Target]) -> Moment {
    let found = targets
        .iter()
        .find(|target| second_of(&target.target_urn) == Some(second));
    let mut counts: Vec<ValueCount> = found
        .and_then(|target| target.interaction_counts.as_ref())
        .into_iter()
        .flatten()
        .filter(|count| count.count > 0)
        .map(|count| ValueCount {
            value_urn: count.interaction_type_value_urn.clone(),
            count: count.count,
        })
        .collect();
    counts.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.value_urn.cmp(&right.value_urn))
    });
    Moment {
        second,
        urn: sc_mobile::reactions::moment_urn(track, second),
        total: counts.iter().map(|count| count.count).sum(),
        counts,
        own: found.and_then(|target| target.user_interaction.clone()),
    }
}

fn second_of(target_urn: &str) -> Option<u64> {
    target_urn.rsplit(':').next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const TRACK: &str = "soundcloud:tracks:7";

    fn body(value: serde_json::Value) -> Vec<u8> {
        value.to_string().into_bytes()
    }

    #[test]
    fn the_value_list_is_the_client_set_with_its_emoji() {
        assert_eq!(
            serde_json::to_value(value_list()).ok(),
            Some(json!([
                { "valueUrn": "sc:interactiontypevalue:1f525", "emoji": "\u{1F525}" },
                { "valueUrn": "sc:interactiontypevalue:1f44f", "emoji": "\u{1F44F}" },
                { "valueUrn": "sc:interactiontypevalue:1f979", "emoji": "\u{1F979}" },
            ]))
        );
    }

    #[test]
    fn counts_and_the_own_reaction_are_mapped_per_requested_moment() {
        let reply = body(json!({
            "data": { "targetInteractions": [
                {
                    "targetUrn": "ts:5",
                    "userInteraction": "sc:interactiontypevalue:1f525",
                    "interactionCounts": [
                        { "interactionTypeValueUrn": "sc:interactiontypevalue:1f60d", "count": 2 },
                        { "interactionTypeValueUrn": "sc:interactiontypevalue:1f525", "count": 3 },
                        { "interactionTypeValueUrn": "sc:interactiontypevalue:1f44f", "count": 0 }
                    ]
                },
                { "targetUrn": "ts:9", "userInteraction": null, "interactionCounts": null }
            ] }
        }));

        let mapped = moments(&reply, TRACK, &[5, 9, 11]);

        assert_eq!(
            mapped,
            Some(vec![
                Moment {
                    second: 5,
                    urn: "soundcloud:tracks:7:moments:5".to_owned(),
                    total: 5,
                    counts: vec![
                        ValueCount {
                            value_urn: "sc:interactiontypevalue:1f525".to_owned(),
                            count: 3
                        },
                        ValueCount {
                            value_urn: "sc:interactiontypevalue:1f60d".to_owned(),
                            count: 2
                        },
                    ],
                    own: Some("sc:interactiontypevalue:1f525".to_owned()),
                },
                Moment {
                    second: 9,
                    urn: "soundcloud:tracks:7:moments:9".to_owned(),
                    total: 0,
                    counts: vec![],
                    own: None,
                },
                Moment {
                    second: 11,
                    urn: "soundcloud:tracks:7:moments:11".to_owned(),
                    total: 0,
                    counts: vec![],
                    own: None,
                },
            ])
        );
    }

    #[test]
    fn a_graphql_error_or_a_foreign_body_is_not_a_reading() {
        let failed = body(json!({ "data": null, "errors": [{ "message": "boom" }] }));
        let empty = body(json!({ "data": { "targetInteractions": null } }));

        assert_eq!(moments(&failed, TRACK, &[5]), None);
        assert_eq!(moments(b"<html>", TRACK, &[5]), None);
        assert_eq!(moments(b"{}", TRACK, &[5]), None);
        assert_eq!(
            moments(&empty, TRACK, &[5]).map(|found| found.len()),
            Some(1)
        );
    }
}
