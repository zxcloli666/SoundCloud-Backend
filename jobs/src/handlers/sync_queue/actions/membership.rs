use serde_json::{Map, Value, json};

pub const ACTION_TYPE: &str = "playlist_membership";
const MAX_TRACKS: usize = 5000;

#[derive(Debug, PartialEq, Eq)]
pub struct MembershipApply {
    tracks: Vec<String>,
    pub fingerprint: String,
    pub reconcile_generation: i64,
}

impl MembershipApply {
    pub fn parse(payload: &Value) -> Result<Self, &'static str> {
        let fields = payload.as_object().ok_or("expected a membership object")?;
        let tracks = fields
            .get("tracks")
            .and_then(Value::as_array)
            .ok_or("membership payload has no track list")?;
        if tracks.len() > MAX_TRACKS {
            return Err("membership payload exceeds the track cap");
        }
        let mut seen = std::collections::HashSet::with_capacity(tracks.len());
        let mut ordered = Vec::with_capacity(tracks.len());
        for track in tracks {
            let id = track.as_str().ok_or("track ids must be strings")?;
            if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err("track ids must be digits");
            }
            if !seen.insert(id) {
                return Err("membership payload repeats a track");
            }
            ordered.push(id.to_owned());
        }
        let fingerprint = fields
            .get("fingerprint")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or("membership payload has no fingerprint")?
            .to_owned();
        let reconcile_generation = fields
            .get("reconcile_generation")
            .and_then(Value::as_i64)
            .filter(|generation| *generation > 0)
            .ok_or("membership payload has no reconcile generation")?;
        Ok(Self {
            tracks: ordered,
            fingerprint,
            reconcile_generation,
        })
    }

    pub fn body(&self) -> Value {
        let mut playlist = Map::new();
        playlist.insert(
            "tracks".into(),
            Value::Array(
                self.tracks
                    .iter()
                    .map(|id| json!({ "id": id.parse::<i64>().unwrap_or_default() }))
                    .collect(),
            ),
        );
        json!({ "playlist": Value::Object(playlist) })
    }

    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(tracks: Value) -> Value {
        json!({ "tracks": tracks, "fingerprint": "abc", "reconcile_generation": 7 })
    }

    #[test]
    fn a_valid_payload_keeps_the_requested_order() {
        let apply = MembershipApply::parse(&payload(json!(["30", "10", "20"]))).expect("parses");
        assert_eq!(apply.len(), 3);
        assert_eq!(
            apply.body(),
            json!({"playlist": {"tracks": [{"id": 30}, {"id": 10}, {"id": 20}]}})
        );
    }

    #[test]
    fn an_empty_list_is_a_legitimate_request_to_clear_the_playlist() {
        let apply = MembershipApply::parse(&payload(json!([]))).expect("parses");
        assert!(apply.is_empty());
        assert_eq!(apply.body(), json!({"playlist": {"tracks": []}}));
    }

    #[test]
    fn a_repeated_track_is_rejected_instead_of_silently_deduplicated() {
        assert_eq!(
            MembershipApply::parse(&payload(json!(["10", "10"]))),
            Err("membership payload repeats a track")
        );
    }

    #[test]
    fn only_numeric_ids_are_accepted() {
        for bad in [json!(["soundcloud:tracks:10"]), json!([10]), json!([""])] {
            assert!(MembershipApply::parse(&payload(bad)).is_err());
        }
    }

    #[test]
    fn a_payload_without_its_fences_is_rejected() {
        assert!(MembershipApply::parse(&json!({"tracks": ["10"]})).is_err());
        assert!(
            MembershipApply::parse(&json!({"tracks": ["10"], "fingerprint": "abc"})).is_err(),
            "a payload without a generation cannot be fenced"
        );
        assert!(
            MembershipApply::parse(
                &json!({"tracks": ["10"], "fingerprint": "", "reconcile_generation": 1})
            )
            .is_err()
        );
    }

    #[test]
    fn an_oversized_list_is_refused_before_it_reaches_soundcloud() {
        let many: Vec<Value> = (0..MAX_TRACKS + 1).map(|n| json!(n.to_string())).collect();
        assert_eq!(
            MembershipApply::parse(&payload(Value::Array(many))),
            Err("membership payload exceeds the track cap")
        );
    }
}
