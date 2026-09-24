use std::fs;
use std::path::PathBuf;

const READS_HISTORY_NOT_CANDIDATES: &[&str] = &[
    "home_wave/quality_rows.sql",
    "home_wave/recent_artists.sql",
    "s3_verifier/select_verify_rows.sql",
    "service/enrichment/track_meta_by_ids.sql",
    "service/enrichment/filter_track_ids_by_language.sql",
    "smart_wave/graph/load_disliked_artists.sql",
    "smart_wave/graph/load_track_seeds.sql",
    "smart_wave/graph/load_track_seeds_via_album.sql",
    "smart_wave/graph/load_user_seeds.sql",
    "smart_wave/mod/load_track_meta.sql",
    "similar_wave/load_primary_artist_id.sql",
];

fn queries_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("queries/recommendations")
}

fn every_query() -> Vec<(String, String)> {
    let root = queries_root();
    let mut found = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|name| name.to_str()) != Some("sql") {
                continue;
            }
            let Ok(body) = fs::read_to_string(&path) else {
                continue;
            };
            let name = path
                .strip_prefix(&root)
                .unwrap_or(&path)
                .display()
                .to_string();
            found.push((name, body));
        }
    }
    found.sort();
    found
}

fn offers_tracks_to_a_listener(body: &str) -> bool {
    body.contains("FROM tracks") || body.contains("JOIN tracks")
}

#[test]
fn no_wave_query_can_offer_a_track_that_lost_a_merge() {
    let mut unfiltered: Vec<String> = Vec::new();

    for (name, body) in every_query() {
        if !offers_tracks_to_a_listener(&body)
            || READS_HISTORY_NOT_CANDIDATES.contains(&name.as_str())
        {
            continue;
        }
        if !body.contains("superseded_by IS NULL") {
            unfiltered.push(name);
        }
    }

    assert!(
        unfiltered.is_empty(),
        "a merge sets `superseded_by` on the loser and leaves its `sharing` alone, so the loser \
         stays public and these queries hand it to the listener next to the winner; \
         `/search/db/tracks` already refuses it:\n  {}",
        unfiltered.join("\n  ")
    );
}

#[test]
fn the_exempt_list_names_only_queries_that_still_exist() {
    let present: Vec<String> = every_query().into_iter().map(|(name, _)| name).collect();
    let gone: Vec<&&str> = READS_HISTORY_NOT_CANDIDATES
        .iter()
        .filter(|name| !present.contains(&(**name).to_owned()))
        .collect();

    assert!(
        gone.is_empty(),
        "the exemption list outlived the queries it excused, so it now hides whatever took \
         their names: {gone:?}"
    );
}

#[test]
fn the_scan_actually_reaches_the_queries_it_claims_to_check() {
    let checked = every_query()
        .into_iter()
        .filter(|(name, body)| {
            offers_tracks_to_a_listener(body)
                && !READS_HISTORY_NOT_CANDIDATES.contains(&name.as_str())
        })
        .count();

    assert!(
        checked >= 12,
        "only {checked} candidate queries were examined; the wave has more than that, so this \
         guard is reading the wrong directory"
    );
}
