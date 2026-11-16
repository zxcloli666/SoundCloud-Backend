use backend_contracts::pipeline::{MatchCandidate, MatchTarget, MatchTrackData, RpcSource};

use super::similarity::token_set;
use crate::handlers::metadata::Normalizer;

const ARTIST_WEIGHT: f64 = 0.45;
const TITLE_WEIGHT: f64 = 0.45;
const DURATION_WEIGHT: f64 = 0.1;
const UNKNOWN_DURATION_SCORE: f64 = 0.5;
const DURATION_TOLERANCE_S: f64 = 4.0;
const DURATION_LIMIT_S: f64 = 25.0;
const DECISIVE_SCORE: f64 = 0.85;
const DECISIVE_MARGIN: f64 = 0.15;
const NO_MATCH_BELOW: f64 = 0.5;
const TIE_MARGIN: f64 = 0.05;

pub struct Track {
    artists: Vec<String>,
    song: String,
    duration_s: Option<f64>,
}

pub struct Scored<'a> {
    pub candidate: &'a MatchCandidate,
    pub score: f64,
}

impl Track {
    pub fn read(
        normalizer: &Normalizer,
        artist: &str,
        title: &str,
        duration_s: Option<f64>,
    ) -> Self {
        let parts = normalizer.parse_title(title);
        let song = if parts.song.is_empty() {
            title.to_owned()
        } else {
            parts.song
        };
        Self {
            artists: [Some(artist.to_owned()), parts.artist]
                .into_iter()
                .flatten()
                .collect(),
            song,
            duration_s,
        }
    }
}

pub fn rank<'a>(
    normalizer: &Normalizer,
    target: &MatchTarget,
    candidates: &'a [MatchCandidate],
) -> Vec<Scored<'a>> {
    let wanted = normalizer.version_markers(&target.title);
    let target = Track::read(normalizer, &target.artist, &target.title, None);
    let mut ranked: Vec<Scored<'a>> = candidates
        .iter()
        .filter(|candidate| normalizer.version_markers(&candidate.title) == wanted)
        .map(|candidate| Scored {
            candidate,
            score: score(
                &target,
                &Track::read(
                    normalizer,
                    &candidate.artist,
                    &candidate.title,
                    candidate.duration_sec,
                ),
            ),
        })
        .collect();
    ranked.sort_by(|left, right| right.score.total_cmp(&left.score));
    ranked
}

pub fn score(target: &Track, candidate: &Track) -> f64 {
    let artist = target
        .artists
        .iter()
        .flat_map(|wanted| {
            candidate
                .artists
                .iter()
                .map(move |offered| token_set(wanted, offered))
        })
        .fold(0.0, f64::max);
    let title = token_set(&target.song, &candidate.song);
    let duration = duration_closeness(target.duration_s, candidate.duration_s);
    ARTIST_WEIGHT * artist + TITLE_WEIGHT * title + DURATION_WEIGHT * duration
}

pub fn decide(ranked: &[Scored]) -> MatchTrackData {
    let verdict = |match_id, confidence| MatchTrackData {
        match_id,
        confidence,
        source: RpcSource::Deterministic,
    };
    let Some(best) = ranked.first() else {
        return verdict(None, 0.0);
    };
    if best.score < NO_MATCH_BELOW || margin(ranked) < TIE_MARGIN {
        return verdict(None, best.score);
    }
    verdict(Some(best.candidate.id), best.score)
}

pub fn is_borderline(ranked: &[Scored]) -> bool {
    match ranked.first() {
        Some(best) if best.score >= NO_MATCH_BELOW => {
            !(best.score >= DECISIVE_SCORE && margin(ranked) >= DECISIVE_MARGIN)
        }
        _ => false,
    }
}

pub fn duration_closeness(target_s: Option<f64>, candidate_s: Option<f64>) -> f64 {
    let (Some(target_s), Some(candidate_s)) = (target_s, candidate_s) else {
        return UNKNOWN_DURATION_SCORE;
    };
    let difference = (target_s - candidate_s).abs();
    if difference <= DURATION_TOLERANCE_S {
        return 1.0;
    }
    if difference >= DURATION_LIMIT_S {
        return 0.0;
    }
    1.0 - (difference - DURATION_TOLERANCE_S) / (DURATION_LIMIT_S - DURATION_TOLERANCE_S)
}

fn margin(ranked: &[Scored]) -> f64 {
    let best = ranked.first().map_or(0.0, |scored| scored.score);
    let runner_up = ranked.get(1).map_or(0.0, |scored| scored.score);
    best - runner_up
}
