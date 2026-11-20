use std::collections::HashMap;

use sqlx::PgPool;

use super::rank::Candidate;

const UNKNOWN_GENRE_DAMP: f32 = 0.5;
const OTHER_GENRE_DAMP: f32 = 0.2;

pub async fn weigh(pg: &PgPool, wanted: Option<&[String]>, cands: &mut [Candidate]) {
    let Some(wanted) = wanted.filter(|wanted| !wanted.is_empty()) else {
        return;
    };
    if cands.is_empty() {
        return;
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
    for cand in cands {
        let genre = genre_of.get(&cand.sc_track_id.to_string());
        cand.skip_damp *= damp(genre.map(String::as_str), &needles);
    }
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
}
