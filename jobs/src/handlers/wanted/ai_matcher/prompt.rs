use std::collections::HashSet;
use std::time::Duration;

use backend_contracts::pipeline::{MatchCandidate, MatchTarget, MatchTrackData, RpcSource};
use llm_ladder::{Message, Request, Schema, System};
use serde::Deserialize;

use super::rank::Scored;
use crate::handlers::metadata::{quoted, share};

const INSTRUCTIONS: [&str; 3] = [
    "Pick the candidate that is the same recording as the target, or null if none is.",
    "A remix, live, acoustic, instrumental, sped up or slowed version is a different",
    "recording. Candidate artists are often uploader names, not the performer.",
];

#[derive(Debug, Deserialize)]
pub struct Pick {
    match_id: Option<u32>,
    confidence: f64,
}

pub fn request(target: &MatchTarget, shortlist: &[Scored], deadline: Duration) -> Request {
    Request {
        system: System::fixed(INSTRUCTIONS.join("\n")),
        messages: vec![Message::user(candidates(target, shortlist))],
        schema: schema(),
        budget_user: None,
        deadline: Some(deadline),
        plain: None,
        text_field: None,
    }
}

pub fn schema() -> Schema {
    Schema::object([
        ("match_id", Schema::Integer.or_null()),
        ("confidence", Schema::Number),
    ])
}

pub fn read(pick: &Pick, allowed: &HashSet<u32>) -> Option<MatchTrackData> {
    let confidence = share(pick.confidence)?;
    if pick.match_id.is_some_and(|id| !allowed.contains(&id)) {
        return None;
    }
    Some(MatchTrackData {
        match_id: pick.match_id,
        confidence,
        source: RpcSource::Llm,
    })
}

fn candidates(target: &MatchTarget, shortlist: &[Scored]) -> String {
    let mut lines = vec![
        format!(
            "target: artist={} title={}",
            quoted(&target.artist),
            quoted(&target.title)
        ),
        "candidates:".to_owned(),
    ];
    lines.extend(
        shortlist
            .iter()
            .map(|scored| candidate_line(scored.candidate)),
    );
    lines.join("\n")
}

fn candidate_line(candidate: &MatchCandidate) -> String {
    let duration = candidate
        .duration_sec
        .map_or_else(|| "None".to_owned(), |seconds| format!("{seconds:?}"));
    format!(
        "- id={} artist={} title={} duration_sec={duration}",
        candidate.id,
        quoted(&candidate.artist),
        quoted(&candidate.title)
    )
}
