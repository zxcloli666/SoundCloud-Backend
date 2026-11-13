use crate::lyrics::{
    LuaEnvelope, LyricsFailure, LyricsHints, LyricsSources, failure, heuristic_queries,
    lua_candidate, relay_failure,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeniusText {
    pub text: String,
    pub linked: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GeniusTexts {
    Found(Vec<GeniusText>),
    Missing,
    Retry(LyricsFailure),
    InsufficientMetadata,
}

impl LyricsSources {
    pub async fn genius_texts(&self, hints: &LyricsHints) -> GeniusTexts {
        let title = hints.title.trim();
        if title.is_empty() {
            return GeniusTexts::InsufficientMetadata;
        }
        let queries = heuristic_queries(hints.artist.trim(), title);
        match self.lua_envelope(hints, &queries).await {
            Ok(envelope) => genius_texts(envelope),
            Err(error) => GeniusTexts::Retry(failure(&error)),
        }
    }
}

fn genius_texts(envelope: LuaEnvelope) -> GeniusTexts {
    if !matches!(envelope.kind.as_str(), "found" | "empty") {
        return GeniusTexts::Retry(relay_failure(&envelope.failed));
    }
    let texts: Vec<GeniusText> = envelope
        .candidates
        .into_iter()
        .filter_map(lua_candidate)
        .filter(|candidate| candidate.source == "genius")
        .filter_map(|candidate| {
            Some(GeniusText {
                text: candidate.plain_text?,
                linked: candidate.exact,
            })
        })
        .collect();
    let genius_failed = envelope
        .failed
        .iter()
        .any(|failure| failure.starts_with("genius: "));
    if envelope.genius_blocked || (genius_failed && texts.is_empty()) {
        return GeniusTexts::Retry(relay_failure(&envelope.failed));
    }
    if texts.is_empty() {
        return GeniusTexts::Missing;
    }
    GeniusTexts::Found(texts)
}
