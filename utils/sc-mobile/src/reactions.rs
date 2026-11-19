use serde_json::{Value, json};

use crate::request::MobileRequest;

pub const TRACK_REACTION: &str = "sc:interactiontype:trackreaction";
pub const FIRE: &str = "sc:interactiontypevalue:1f525";
pub const CLAP: &str = "sc:interactiontypevalue:1f44f";
pub const CRY: &str = "sc:interactiontypevalue:1f979";

pub struct ReactionValue {
    pub value_urn: &'static str,
    pub emoji: &'static str,
}

pub const VALUES: [ReactionValue; 3] = [
    ReactionValue {
        value_urn: FIRE,
        emoji: "\u{1F525}",
    },
    ReactionValue {
        value_urn: CLAP,
        emoji: "\u{1F44F}",
    },
    ReactionValue {
        value_urn: CRY,
        emoji: "\u{1F979}",
    },
];

pub fn is_value(value_urn: &str) -> bool {
    VALUES.iter().any(|value| value.value_urn == value_urn)
}

pub const UPSERT_INTERACTION: &str = "mutation UpsertInteraction($input: InteractionInput!) {
  upsertInteraction(input: $input) {
    targetUrn
  }
}";

pub const REMOVE_INTERACTION: &str = "mutation RemoveInteraction($input: InteractionInput!) {
  removeInteraction(input: $input)
}";

pub const REACTIONS_PER_TARGET: &str = "query ReactionsPerTarget(
  $interactionTypeUrn: String!,
  $limitPerTarget: Int!,
  $parentUrn: String!,
  $targetUrns: [String!]!
) {
  targetInteractions(
    interactionTypeUrn: $interactionTypeUrn
    parentUrn: $parentUrn
    limitPerTarget: $limitPerTarget
    targetUrns: $targetUrns
  ) {
    userInteraction
    targetUrn
    interactionCounts {
      interactionTypeValueUrn
      count
    }
    user {
      userAvatarUrlTemplate
    }
  }
}";

pub(crate) const LIMIT_PER_TARGET: u32 = 10;

pub struct TrackReaction<'a> {
    pub track_urn: &'a str,
    pub at_second: u64,
    pub value_urn: &'a str,
}

impl TrackReaction<'_> {
    pub fn target_urn(&self) -> String {
        moment(self.at_second)
    }

    fn input(&self) -> Value {
        json!({
            "targetUrn": self.target_urn(),
            "parentUrn": self.track_urn,
            "interactionTypeUrn": TRACK_REACTION,
            "interactionTypeValueUrn": self.value_urn,
        })
    }
}

pub fn add(reaction: &TrackReaction<'_>) -> MobileRequest {
    MobileRequest::mutation(UPSERT_INTERACTION, json!({ "input": reaction.input() }))
}

pub fn remove(reaction: &TrackReaction<'_>) -> MobileRequest {
    MobileRequest::mutation(REMOVE_INTERACTION, json!({ "input": reaction.input() }))
}

pub fn read(track_urn: &str, at_seconds: &[u64]) -> MobileRequest {
    let targets: Vec<String> = at_seconds.iter().copied().map(moment).collect();
    MobileRequest::query(
        REACTIONS_PER_TARGET,
        json!({
            "interactionTypeUrn": TRACK_REACTION,
            "limitPerTarget": LIMIT_PER_TARGET,
            "parentUrn": track_urn,
            "targetUrns": targets,
        }),
    )
}

fn moment(second: u64) -> String {
    format!("ts:{second}")
}

const MOMENT_MARK: &str = ":moments:";

pub fn moment_urn(track_urn: &str, at_second: u64) -> String {
    format!("{track_urn}{MOMENT_MARK}{at_second}")
}

pub fn parse_moment_urn(urn: &str) -> Option<(&str, u64)> {
    let (track, second) = urn.rsplit_once(MOMENT_MARK)?;
    if !track.starts_with("soundcloud:tracks:") {
        return None;
    }
    let at_second = second.parse::<u64>().ok()?;
    (second == at_second.to_string()).then_some((track, at_second))
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
    fn a_track_reaction_targets_a_moment_of_its_track() {
        let reaction = TrackReaction {
            track_urn: "soundcloud:tracks:1",
            at_second: 5,
            value_urn: FIRE,
        };
        let expected = json!({
            "input": {
                "targetUrn": "ts:5",
                "parentUrn": "soundcloud:tracks:1",
                "interactionTypeUrn": "sc:interactiontype:trackreaction",
                "interactionTypeValueUrn": "sc:interactiontypevalue:1f525",
            }
        });

        let added = add(&reaction);
        let removed = remove(&reaction);

        assert_eq!(added.kind(), Kind::Mutation);
        assert_eq!(removed.kind(), Kind::Mutation);
        assert_eq!(variables(&added), Some(expected.clone()));
        assert_eq!(variables(&removed), Some(expected));
    }

    #[test]
    fn a_moment_urn_names_the_track_and_the_second_and_parses_back() {
        let urn = moment_urn("soundcloud:tracks:123", 45);

        assert_eq!(urn, "soundcloud:tracks:123:moments:45");
        assert_eq!(parse_moment_urn(&urn), Some(("soundcloud:tracks:123", 45)));
        for bad in [
            "soundcloud:tracks:123",
            "soundcloud:tracks:123:moments:",
            "soundcloud:tracks:123:moments:-1",
            "soundcloud:tracks:123:moments:045",
            "soundcloud:users:1:moments:5",
            ":moments:5",
        ] {
            assert_eq!(parse_moment_urn(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_value_set_is_the_client_set_in_its_order() {
        let urns: Vec<&str> = VALUES.iter().map(|value| value.value_urn).collect();

        assert_eq!(
            urns,
            [
                "sc:interactiontypevalue:1f525",
                "sc:interactiontypevalue:1f44f",
                "sc:interactiontypevalue:1f979",
            ]
        );
        assert_eq!(VALUES[0].emoji, "\u{1F525}");
        assert!(is_value(CLAP));
        assert!(!is_value("sc:interactiontypevalue:like"));
        assert!(!is_value("1f525"));
    }

    #[test]
    fn reading_reactions_is_a_query_over_moments() {
        let request = read("soundcloud:tracks:1", &[5, 7]);

        assert_eq!(request.kind(), Kind::Query);
        assert_eq!(
            variables(&request).and_then(|variables| variables.get("targetUrns").cloned()),
            Some(json!(["ts:5", "ts:7"]))
        );
    }
}
