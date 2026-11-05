use std::collections::{HashMap, HashSet};

use sqlx::PgPool;
use uuid::Uuid;

use super::artist_tracks::ArtistTopTracks;
use super::cursor::WaveCursor;
use super::graph::Affinity;
use super::rank::{self, Candidate, RankedTrack};

const CEILING_SLACK: f32 = 1e-3;

pub struct Reach<'a> {
    pub affinity: &'a Affinity,
    pub disliked_artists: &'a HashSet<Uuid>,
    pub cursor: &'a WaveCursor,
    pub picks: usize,
    pub artist_cap: usize,
    pub graph_floor: f32,
    pub content_floor: f32,
}

impl Reach<'_> {
    pub fn pick(&self, cands: &[Candidate]) -> Vec<RankedTrack> {
        rank::rank_and_pick(
            cands,
            self.affinity,
            self.disliked_artists,
            self.cursor,
            self.picks,
            self.artist_cap,
            self.graph_floor,
            self.content_floor,
        )
    }

    pub fn split<'r>(&self, ranked: &'r [Uuid]) -> (&'r [Uuid], &'r [Uuid]) {
        ranked.split_at(ranked.len().min(self.picks))
    }

    pub fn tail(&self, rest: &[Uuid], picked: &[RankedTrack]) -> Vec<Uuid> {
        let Some(cut) = picked
            .last()
            .filter(|_| picked.len() >= self.picks)
            .map(|last| last.score)
        else {
            return rest.to_vec();
        };
        rest.iter()
            .copied()
            .filter(|artist| self.ceiling(*artist) >= cut)
            .collect()
    }

    fn ceiling(&self, artist: Uuid) -> f32 {
        let affinity = self.affinity.get(&artist).copied().unwrap_or(0.0);
        let factor = self.graph_floor + (1.0 - self.graph_floor) * affinity.clamp(0.0, 1.0);
        factor * (1.0 + CEILING_SLACK)
    }
}

pub async fn read(
    tops: &ArtistTopTracks,
    pg: &PgPool,
    artists: &[Uuid],
    ranked: &[Uuid],
    exclude: &[String],
    per_artist: i64,
    room: usize,
) -> Vec<(u64, Uuid)> {
    if artists.is_empty() || room == 0 {
        return Vec::new();
    }
    let all = per_artist.saturating_mul(i64::try_from(artists.len()).unwrap_or(i64::MAX));
    let rows = tops.collect(pg, artists, exclude, per_artist, all).await;
    in_rank_order(rows, ranked, room)
}

pub fn in_rank_order(mut rows: Vec<(u64, Uuid)>, ranked: &[Uuid], room: usize) -> Vec<(u64, Uuid)> {
    let rank: HashMap<Uuid, usize> = ranked
        .iter()
        .enumerate()
        .map(|(index, artist)| (*artist, index))
        .collect();
    rows.sort_by_key(|(_, artist)| rank.get(artist).copied().unwrap_or(usize::MAX));
    rows.truncate(room);
    rows
}

pub fn attributed(rows: &[(u64, Uuid)], known: &HashSet<u64>) -> HashMap<u64, Option<Uuid>> {
    let mut artist_of = HashMap::new();
    for (track, artist) in rows {
        if !known.contains(track) {
            artist_of.entry(*track).or_insert(Some(*artist));
        }
    }
    artist_of
}
