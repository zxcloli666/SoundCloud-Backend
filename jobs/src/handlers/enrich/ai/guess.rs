use super::ResolveArtistRequest;
use crate::handlers::metadata::{Normalizer, casefold, is_reupload_channel, unique};

pub const REFINE_BELOW_CONFIDENCE: f64 = 0.8;
const METADATA_CONFIDENCE: f64 = 0.9;
const TITLE_CONFIDENCE: f64 = 0.7;
const UPLOADER_CONFIDENCE: f64 = 0.5;
const UNKNOWN_CONFIDENCE: f64 = 0.3;

#[derive(Clone, Debug, PartialEq)]
pub struct Guess {
    pub primary_artist: Option<String>,
    pub featured: Vec<String>,
    pub confidence: f64,
}

impl Guess {
    pub fn is_confident(&self) -> bool {
        self.confidence >= REFINE_BELOW_CONFIDENCE
    }
}

pub fn deterministic(normalizer: &Normalizer, track: &ResolveArtistRequest) -> Guess {
    let parts = normalizer.parse_title(&track.title);
    let credits = normalizer.split_credits(track.metadata_artist.as_deref().unwrap_or(""));
    let uploader = track.uploader.as_deref().unwrap_or("");
    let named_uploader = normalizer.tidy(uploader);
    let (primary, confidence, featured) = if let Some(main) = credits.main {
        let featured = credits.featured.into_iter().chain(parts.featured).collect();
        (Some(main), METADATA_CONFIDENCE, featured)
    } else if let Some(artist) = parts.artist {
        (Some(artist), TITLE_CONFIDENCE, parts.featured)
    } else if !named_uploader.is_empty() && !is_reupload_channel(uploader) {
        (Some(named_uploader), UPLOADER_CONFIDENCE, parts.featured)
    } else {
        (None, UNKNOWN_CONFIDENCE, parts.featured)
    };
    let primary_key = primary.as_deref().map(casefold);
    Guess {
        featured: unique(featured)
            .into_iter()
            .filter(|name| Some(casefold(name)) != primary_key)
            .collect(),
        primary_artist: primary,
        confidence,
    }
}
