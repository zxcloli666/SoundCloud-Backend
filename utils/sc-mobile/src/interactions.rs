use serde_json::{Value, json};

use crate::reactions::{
    LIMIT_PER_TARGET, REACTIONS_PER_TARGET, REMOVE_INTERACTION, UPSERT_INTERACTION,
};
use crate::request::MobileRequest;

pub const REACTION: &str = "sc:interactiontype:reaction";
pub const LIKE: &str = "sc:interactiontypevalue:like";

pub struct Interaction<'a> {
    pub type_urn: &'a str,
    pub value_urn: Option<&'a str>,
    pub target_urn: &'a str,
    pub parent_urn: &'a str,
}

pub fn comment_like<'a>(comment_urn: &'a str, track_urn: &'a str) -> Interaction<'a> {
    Interaction {
        type_urn: REACTION,
        value_urn: Some(LIKE),
        target_urn: comment_urn,
        parent_urn: track_urn,
    }
}

pub fn add(interaction: &Interaction<'_>) -> MobileRequest {
    MobileRequest::mutation(UPSERT_INTERACTION, json!({ "input": input(interaction) }))
}

pub fn remove(interaction: &Interaction<'_>) -> MobileRequest {
    MobileRequest::mutation(REMOVE_INTERACTION, json!({ "input": input(interaction) }))
}

pub fn read(interaction: &Interaction<'_>) -> MobileRequest {
    MobileRequest::query(
        REACTIONS_PER_TARGET,
        json!({
            "interactionTypeUrn": interaction.type_urn,
            "limitPerTarget": LIMIT_PER_TARGET,
            "parentUrn": interaction.parent_urn,
            "targetUrns": [interaction.target_urn],
        }),
    )
}

fn input(interaction: &Interaction<'_>) -> Value {
    let mut input = json!({
        "targetUrn": interaction.target_urn,
        "parentUrn": interaction.parent_urn,
        "interactionTypeUrn": interaction.type_urn,
    });
    if let Some(value) = interaction.value_urn {
        input["interactionTypeValueUrn"] = json!(value);
    }
    input
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::Kind;

    fn variables(request: &MobileRequest) -> Option<Value> {
        request
            .body()
            .and_then(|body| serde_json::from_slice::<Value>(body).ok())
            .and_then(|body| body.get("variables").cloned())
    }

    #[test]
    fn a_comment_like_targets_the_comment_under_its_track() {
        let interaction = comment_like("soundcloud:comments:9", "soundcloud:tracks:1");

        assert_eq!(
            variables(&add(&interaction)),
            Some(json!({
                "input": {
                    "targetUrn": "soundcloud:comments:9",
                    "parentUrn": "soundcloud:tracks:1",
                    "interactionTypeUrn": "sc:interactiontype:reaction",
                    "interactionTypeValueUrn": "sc:interactiontypevalue:like",
                }
            }))
        );
    }

    #[test]
    fn reading_an_interaction_is_a_query_on_its_target() {
        let request = read(&comment_like(
            "soundcloud:comments:9",
            "soundcloud:tracks:1",
        ));

        assert_eq!(request.kind(), Kind::Query);
        assert_eq!(
            variables(&request),
            Some(json!({
                "interactionTypeUrn": "sc:interactiontype:reaction",
                "limitPerTarget": 10,
                "parentUrn": "soundcloud:tracks:1",
                "targetUrns": ["soundcloud:comments:9"],
            }))
        );
    }
}
