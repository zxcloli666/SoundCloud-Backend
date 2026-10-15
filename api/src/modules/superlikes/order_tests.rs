use chrono::{TimeZone, Utc};

use super::*;

fn listed(entries: &[(&str, Option<i64>)]) -> Vec<Superlike> {
    entries
        .iter()
        .enumerate()
        .map(|(i, (id, rank))| Superlike {
            sc_track_id: (*id).to_owned(),
            created_at: Utc.timestamp_opt(1_000 - i as i64, 0).unwrap(),
            rank: *rank,
        })
        .collect()
}

fn order_after(listed: &[Superlike], changed: &[Placed]) -> Vec<String> {
    let mut ranks: Vec<(i64, String)> = listed
        .iter()
        .map(|s| {
            let rank = changed
                .iter()
                .find(|p| p.sc_track_id == s.sc_track_id)
                .map(|p| p.rank)
                .or(s.rank)
                .expect("every superlike is ranked after a move");
            (rank, s.sc_track_id.clone())
        })
        .collect();
    ranks.sort();
    ranks.into_iter().map(|(_, id)| id).collect()
}

#[test]
fn the_first_move_ranks_every_superlike_and_keeps_the_newest_on_top() {
    let listed = listed(&[("3", None), ("2", None), ("1", None)]);

    let changed = arrange(&listed, "1", Target::After("3")).unwrap();

    assert_eq!(changed.len(), 3);
    assert_eq!(order_after(&listed, &changed), vec!["3", "1", "2"]);
}

#[test]
fn a_new_superlike_stays_above_the_ordered_ones_and_only_it_and_the_moved_track_are_written() {
    let listed = listed(&[
        ("9", None),
        ("1", Some(STEP)),
        ("2", Some(2 * STEP)),
        ("3", Some(3 * STEP)),
    ]);

    let changed = arrange(&listed, "1", Target::Bottom).unwrap();

    let written: Vec<&str> = changed.iter().map(|p| p.sc_track_id.as_str()).collect();
    assert_eq!(written, vec!["9", "1"]);
    assert_eq!(order_after(&listed, &changed), vec!["9", "2", "3", "1"]);
}

#[test]
fn moves_to_the_top_and_between_neighbours_split_the_gap() {
    let listed = listed(&[("1", Some(0)), ("2", Some(10)), ("3", Some(20))]);

    let top = arrange(&listed, "3", Target::Top).unwrap();
    assert_eq!(order_after(&listed, &top), vec!["3", "1", "2"]);

    let middle = arrange(&listed, "3", Target::After("1")).unwrap();
    assert_eq!(middle.len(), 1);
    assert_eq!(middle[0].rank, 5);
}

#[test]
fn a_full_gap_renumbers_the_whole_list() {
    let listed = listed(&[("1", Some(1)), ("2", Some(2)), ("3", Some(3))]);

    let changed = arrange(&listed, "3", Target::After("1")).unwrap();

    assert_eq!(order_after(&listed, &changed), vec!["1", "3", "2"]);
    let mut ranks: Vec<i64> = changed.iter().map(|p| p.rank).collect();
    ranks.sort();
    assert_eq!(ranks, vec![STEP, 2 * STEP, 3 * STEP]);
}

#[test]
fn a_rank_at_the_floor_renumbers_instead_of_overflowing() {
    let listed = listed(&[("9", None), ("1", Some(i64::MIN + 1))]);

    let changed = arrange(&listed, "1", Target::Top).unwrap();

    assert_eq!(order_after(&listed, &changed), vec!["1", "9"]);
}

#[test]
fn unknown_tracks_and_anchors_are_a_coded_404_and_a_self_anchor_is_refused() {
    let listed = listed(&[("1", None), ("2", None)]);

    let unknown = arrange(&listed, "7", Target::Top).unwrap_err();
    assert_eq!(unknown.public_code(), "superlike_not_found");
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    let anchor = arrange(&listed, "1", Target::After("7")).unwrap_err();
    assert_eq!(anchor.public_code(), "superlike_not_found");
    let itself = arrange(&listed, "1", Target::After("1")).unwrap_err();
    assert_eq!(itself.status(), StatusCode::BAD_REQUEST);
}
