use std::collections::{HashMap, HashSet};

use sqlx::PgPool;
use uuid::Uuid;

use super::rank::{Candidate, RankedTrack};

const UNKNOWN_GENRE_DAMP: f32 = 0.5;
const OTHER_GENRE_DAMP: f32 = 0.2;
const POOL_PER_VARIANT: i64 = 60;
const SHARE_PERCENT: usize = 75;

pub async fn weigh(
    pg: &PgPool,
    wanted: Option<&[String]>,
    cands: &mut [Candidate],
) -> HashSet<u64> {
    let Some(wanted) = wanted.filter(|wanted| !wanted.is_empty()) else {
        return HashSet::new();
    };
    if cands.is_empty() {
        return HashSet::new();
    }
    let needles = needles(wanted);
    let ids: Vec<String> = cands.iter().map(|c| c.sc_track_id.to_string()).collect();
    let rows = sqlx::query_file!(
        "queries/recommendations/smart_wave/mod/load_track_genres.sql",
        &ids
    )
    .fetch_all(pg)
    .await
    .unwrap_or_default();
    let genre_of: HashMap<String, String> = rows
        .into_iter()
        .filter_map(|r| r.genre.map(|genre| (r.sc_track_id, genre)))
        .collect();
    let mut matched = HashSet::new();
    for cand in cands {
        let genre = genre_of.get(&cand.sc_track_id.to_string());
        let weight = damp(genre.map(String::as_str), &needles);
        if weight == 1.0 {
            matched.insert(cand.sc_track_id);
        }
        cand.skip_damp *= weight;
    }
    matched
}

pub async fn pool(
    pg: &PgPool,
    wanted: Option<&[String]>,
    exclude: &[String],
) -> Vec<(u64, Option<Uuid>)> {
    let Some(wanted) = wanted.filter(|wanted| !wanted.is_empty()) else {
        return Vec::new();
    };
    let names: Vec<String> = wanted.iter().flat_map(|key| variants(key)).collect();
    let rows = sqlx::query_file!(
        "queries/recommendations/smart_wave/mod/genre_pool.sql",
        &names,
        exclude,
        POOL_PER_VARIANT
    )
    .fetch_all(pg)
    .await
    .unwrap_or_default();
    rows.into_iter()
        .filter_map(|r| Some((r.sc_track_id.parse().ok()?, r.primary_artist_id)))
        .collect()
}

pub fn lead(ranked: Vec<RankedTrack>, matched: &HashSet<u64>, limit: usize) -> Vec<RankedTrack> {
    if matched.is_empty() {
        return ranked;
    }
    let room = limit - (limit * SHARE_PERCENT).div_ceil(100).min(limit);
    let mut kept = Vec::with_capacity(ranked.len());
    let mut held = Vec::new();
    let mut others = 0;
    for track in ranked {
        if matched.contains(&track.sc_track_id) {
            kept.push(track);
        } else if others < room {
            others += 1;
            kept.push(track);
        } else {
            held.push(track);
        }
    }
    kept.extend(held);
    kept
}

pub fn variants(key: &str) -> Vec<String> {
    let names: &[&str] = match key {
        "lofi" => &["lofi", "lo-fi", "lofi hip hop", "lo-fi hip hop"],
        "house" => &[
            "house",
            "deep house",
            "tech house",
            "progressive house",
            "electro house",
            "tropical house",
        ],
        "phonk" => &["phonk", "drift phonk", "brazilian phonk"],
        "ambient" => &["ambient", "dark ambient"],
        "rnb" => &["r&b", "r&b & soul", "rnb", "r&b/soul", "alternative r&b"],
        "trap" => &["trap", "trap & bass", "latin trap"],
        "jazz" => &["jazz", "jazz & blues", "nu jazz", "smooth jazz"],
        "techno" => &["techno", "hard techno", "minimal techno", "melodic techno"],
        "indie" => &["indie", "indie rock", "indie pop", "indie folk"],
        "soul" => &["soul", "neo soul", "r&b & soul"],
        "dnb" => &[
            "dnb",
            "drum & bass",
            "drum and bass",
            "drum n bass",
            "drum&bass",
            "liquid drum & bass",
            "neurofunk",
        ],
        _ => return vec![key.to_lowercase()],
    };
    names.iter().map(|name| name.to_string()).collect()
}

pub fn damp(genre: Option<&str>, needles: &[String]) -> f32 {
    match genre.map(squash) {
        Some(genre) if genre.is_empty() => UNKNOWN_GENRE_DAMP,
        Some(genre) if needles.iter().any(|needle| genre.contains(needle.as_str())) => 1.0,
        Some(_) => OTHER_GENRE_DAMP,
        None => UNKNOWN_GENRE_DAMP,
    }
}

pub fn needles(wanted: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for key in wanted {
        let key = squash(key);
        match key.as_str() {
            "dnb" => out.extend(["dnb", "drumandbass", "drumnbass"].map(String::from)),
            "rnb" => out.extend(["rnb", "randb"].map(String::from)),
            "" => {}
            _ => out.push(key),
        }
    }
    out
}

fn squash(raw: &str) -> String {
    raw.to_lowercase()
        .replace('&', "and")
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wanted(keys: &[&str]) -> Vec<String> {
        needles(&keys.iter().map(|k| k.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn a_chosen_genre_keeps_full_weight() {
        let n = wanted(&["house"]);
        assert_eq!(damp(Some("Deep House"), &n), 1.0);
        assert_eq!(damp(Some("house"), &n), 1.0);
    }

    #[test]
    fn another_genre_is_damped_not_dropped() {
        let n = wanted(&["jazz"]);
        let damped = damp(Some("Techno"), &n);
        assert!(damped > 0.0 && damped < 1.0);
    }

    #[test]
    fn a_track_without_a_genre_sits_between_match_and_miss() {
        let n = wanted(&["jazz"]);
        let unknown = damp(None, &n);
        assert!(unknown > damp(Some("Techno"), &n));
        assert!(unknown < damp(Some("Jazz"), &n));
        assert_eq!(damp(Some("  "), &n), unknown);
    }

    #[test]
    fn app_keys_match_soundcloud_spellings() {
        assert_eq!(damp(Some("Drum & Bass"), &wanted(&["dnb"])), 1.0);
        assert_eq!(damp(Some("Drum n Bass"), &wanted(&["dnb"])), 1.0);
        assert_eq!(damp(Some("R&B & Soul"), &wanted(&["rnb"])), 1.0);
        assert_eq!(damp(Some("Lo-Fi Hip Hop"), &wanted(&["lofi"])), 1.0);
    }

    #[test]
    fn any_of_several_genres_matches() {
        let n = wanted(&["jazz", "soul"]);
        assert_eq!(damp(Some("Neo Soul"), &n), 1.0);
        assert_eq!(damp(Some("Jazz"), &n), 1.0);
        assert!(damp(Some("Metal"), &n) < 1.0);
    }

    fn picked_order(genres: [Option<&str>; 3], want: &[&str]) -> Vec<u64> {
        use crate::modules::recommendations::smart_wave::cursor::{SeedKind, WaveCursor};
        use crate::modules::recommendations::smart_wave::rank::rank_and_pick;
        let n = wanted(want);
        let contents = [0.9_f32, 0.8, 0.7];
        let cands: Vec<Candidate> = genres
            .iter()
            .enumerate()
            .map(|(i, genre)| Candidate {
                sc_track_id: i as u64 + 1,
                artist: None,
                content: contents[i],
                skip_damp: damp(*genre, &n),
            })
            .collect();
        let cursor = WaveCursor::new(SeedKind::User, "test".into());
        rank_and_pick(
            &cands,
            &Default::default(),
            &Default::default(),
            &cursor,
            3,
            2,
            0.12,
            0.55,
        )
        .iter()
        .map(|p| p.sc_track_id)
        .collect()
    }

    #[test]
    fn matching_tracks_lead_the_wave_even_with_weaker_content() {
        let order = picked_order([Some("Metal"), Some("Jazz"), Some("Jazz")], &["jazz"]);
        assert_eq!(order, vec![2, 3, 1]);
    }

    #[test]
    fn a_wave_without_any_match_still_fills() {
        let order = picked_order([Some("Metal"), Some("Pop"), None], &["jazz"]);
        assert_eq!(order.len(), 3);
    }

    fn ranked(id: u64, score: f32) -> RankedTrack {
        RankedTrack {
            sc_track_id: id,
            score,
            artist: None,
        }
    }

    fn wave(matching: u64, others: u64, limit: usize) -> Vec<u64> {
        let mut all: Vec<RankedTrack> = (0..others).map(|i| ranked(100 + i, 0.9)).collect();
        all.extend((0..matching).map(|i| ranked(i, 0.4)));
        all.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
        let matched: HashSet<u64> = (0..matching).collect();
        lead(all, &matched, limit)
            .into_iter()
            .take(limit)
            .map(|t| t.sc_track_id)
            .collect()
    }

    #[test]
    fn a_pool_with_few_taste_matches_still_gives_a_genre_majority() {
        let ids = wave(30, 40, 20);
        assert_eq!(ids.len(), 20);
        let genre = ids.iter().filter(|id| **id < 100).count();
        assert!(genre >= 15, "only {genre} of 20 matched");
        assert!(genre < 20, "the rest of the taste must still appear");
    }

    #[test]
    fn too_few_genre_tracks_are_filled_from_the_rest() {
        let ids = wave(3, 40, 20);
        assert_eq!(ids.len(), 20);
        assert_eq!(ids.iter().filter(|id| **id < 100).count(), 3);
    }

    #[test]
    fn no_genre_choice_keeps_the_ranking() {
        let all = vec![ranked(1, 0.9), ranked(2, 0.8)];
        let ids: Vec<u64> = lead(all, &HashSet::new(), 2)
            .iter()
            .map(|t| t.sc_track_id)
            .collect();
        assert_eq!(ids, vec![1, 2]);
    }

    #[test]
    fn every_app_genre_has_the_lowercase_names_the_index_holds() {
        for key in ["dnb", "rnb", "house", "hyperpop", "Unknown"] {
            assert!(
                variants(key)
                    .iter()
                    .all(|name| *name == name.to_lowercase())
            );
        }
        assert!(variants("dnb").contains(&"drum & bass".to_string()));
    }
}
