use std::collections::HashMap;
use std::sync::Arc;

use backend_contracts::pipeline::{
    ResolveArtistData, ResolveArtistRequest, ResolvedAlbum, RpcSource,
};
use serde::Deserialize;

use crate::handlers::metadata::{find_name, share, unique};

const EARLIEST_YEAR: i64 = 1900;
const LATEST_YEAR: i64 = 2100;

#[derive(Debug, Deserialize)]
pub struct ModelAnswer {
    primary_artist: Option<String>,
    featured: Vec<Option<String>>,
    producers: Vec<Option<String>>,
    remixers: Vec<Option<String>>,
    album: Option<ModelAlbum>,
    confidence: f64,
}

#[derive(Debug, Deserialize)]
struct ModelAlbum {
    title: Option<String>,
    year: Option<i64>,
    primary_artist: Option<String>,
}

#[derive(Clone)]
pub struct Grounding {
    guessed_primary: bool,
    sources: Arc<[String; 4]>,
}

impl Grounding {
    pub fn new(track: &ResolveArtistRequest, guessed_primary: bool) -> Self {
        let text = |value: &Option<String>| value.clone().unwrap_or_default();
        Self {
            guessed_primary,
            sources: Arc::new([
                track.title.clone(),
                text(&track.uploader),
                text(&track.metadata_artist),
                text(&track.description),
            ]),
        }
    }

    pub fn answer(&self, reply: &ModelAnswer) -> Option<ResolveArtistData> {
        self.grounded(read(reply)?)
    }

    fn grounded(&self, answer: ResolveArtistData) -> Option<ResolveArtistData> {
        if self.guessed_primary && answer.primary_artist.is_none() {
            return None;
        }
        let mut written = HashMap::new();
        for name in names(&answer) {
            written.insert(name.clone(), self.spelling(name)?);
        }
        let respell = |name: &String| written.get(name).cloned();
        let album = match &answer.album {
            Some(album) => Some(ResolvedAlbum {
                title: respell(&album.title)?,
                year: album.year,
                primary_artist: album.primary_artist.as_ref().and_then(respell),
            }),
            None => None,
        };
        Some(ResolveArtistData {
            primary_artist: answer.primary_artist.as_ref().and_then(respell),
            featured: unique(answer.featured.iter().filter_map(respell)),
            producers: unique(answer.producers.iter().filter_map(respell)),
            remixers: unique(answer.remixers.iter().filter_map(respell)),
            album,
            confidence: answer.confidence,
            source: RpcSource::Llm,
        })
    }

    fn spelling(&self, name: &str) -> Option<String> {
        self.sources
            .iter()
            .find_map(|source| find_name(source, name))
    }
}

fn read(reply: &ModelAnswer) -> Option<ResolveArtistData> {
    Some(ResolveArtistData {
        primary_artist: text(reply.primary_artist.as_deref()),
        featured: texts(&reply.featured),
        producers: texts(&reply.producers),
        remixers: texts(&reply.remixers),
        album: match &reply.album {
            Some(album) => Some(read_album(album)?),
            None => None,
        },
        confidence: share(reply.confidence)?,
        source: RpcSource::Llm,
    })
}

fn read_album(album: &ModelAlbum) -> Option<ResolvedAlbum> {
    let year = match album.year {
        Some(year) if (EARLIEST_YEAR..=LATEST_YEAR).contains(&year) => i32::try_from(year).ok(),
        Some(_) => return None,
        None => None,
    };
    Some(ResolvedAlbum {
        title: text(album.title.as_deref())?,
        year,
        primary_artist: text(album.primary_artist.as_deref()),
    })
}

fn names(answer: &ResolveArtistData) -> Vec<&String> {
    let album = answer
        .album
        .iter()
        .flat_map(|album| [Some(&album.title), album.primary_artist.as_ref()]);
    answer
        .primary_artist
        .iter()
        .chain(&answer.featured)
        .chain(&answer.producers)
        .chain(&answer.remixers)
        .chain(album.flatten())
        .collect()
}

fn text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn texts(values: &[Option<String>]) -> Vec<String> {
    unique(values.iter().filter_map(|value| text(value.as_deref())))
}
