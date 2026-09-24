use uuid::Uuid;

use super::reduce::{
    Boundary, Intent, MOVE_ANCHOR_INVERTED, MOVE_ANCHOR_MISSING, MOVE_TARGET_MISSING, Operation,
    Placement, REORDER_TARGETS_MISSING, ReductionOutcome, Resolution, reduce,
};

fn move_between(sequence: i64, track_id: &str, left: &str, right: &str) -> Operation {
    operation(
        sequence,
        Intent::Move {
            track_id: track_id.to_owned(),
            placement: Placement::Anchored {
                left: Some(left.to_owned()),
                right: Some(right.to_owned()),
            },
        },
    )
}

fn ids(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn operation(sequence: i64, intent: Intent) -> Operation {
    Operation {
        operation_id: Uuid::from_u128(sequence as u128),
        sequence,
        intent,
    }
}

fn add_back(sequence: i64, track_id: &str) -> Operation {
    operation(
        sequence,
        Intent::Add {
            track_id: track_id.to_owned(),
            placement: Placement::Boundary(Boundary::Back),
        },
    )
}

fn add_after(sequence: i64, track_id: &str, left: &str) -> Operation {
    operation(
        sequence,
        Intent::Add {
            track_id: track_id.to_owned(),
            placement: Placement::Anchored {
                left: Some(left.to_owned()),
                right: None,
            },
        },
    )
}

fn remove(sequence: i64, track_id: &str) -> Operation {
    operation(
        sequence,
        Intent::Remove {
            track_id: track_id.to_owned(),
        },
    )
}

fn move_after(sequence: i64, track_id: &str, left: &str) -> Operation {
    operation(
        sequence,
        Intent::Move {
            track_id: track_id.to_owned(),
            placement: Placement::Anchored {
                left: Some(left.to_owned()),
                right: None,
            },
        },
    )
}

fn move_front(sequence: i64, track_id: &str) -> Operation {
    operation(
        sequence,
        Intent::Move {
            track_id: track_id.to_owned(),
            placement: Placement::Boundary(Boundary::Front),
        },
    )
}

fn reorder(sequence: i64, ordered: &[&str]) -> Operation {
    operation(
        sequence,
        Intent::Reorder {
            ordered_track_ids: ids(ordered),
        },
    )
}

fn resolutions(reduction: &super::reduce::Reduction) -> Vec<Resolution> {
    reduction
        .operations
        .iter()
        .map(|operation| operation.resolution)
        .collect()
}

#[test]
fn a_local_addition_rebases_onto_unseen_remote_additions() {
    let remote = ids(&["a", "d", "b"]);
    let reduction = reduce(&remote, &[add_back(1, "x")], 0);

    assert_eq!(reduction.candidate, ids(&["a", "d", "b", "x"]));
    assert_eq!(resolutions(&reduction), vec![Resolution::Pending]);
    assert_eq!(reduction.outcome, ReductionOutcome::Rebased);
    assert_eq!(reduction.committed_through_sequence, 0);
}

#[test]
fn an_addition_the_remote_already_carries_is_committed_without_duplicating() {
    let remote = ids(&["a", "x", "b"]);
    let reduction = reduce(&remote, &[add_back(1, "x")], 0);

    assert_eq!(reduction.candidate, remote);
    assert_eq!(resolutions(&reduction), vec![Resolution::Committed]);
    assert_eq!(reduction.outcome, ReductionOutcome::Converged);
    assert_eq!(reduction.committed_through_sequence, 1);
}

#[test]
fn a_removal_the_remote_already_applied_is_committed() {
    let remote = ids(&["a", "b"]);
    let reduction = reduce(&remote, &[remove(1, "x")], 0);

    assert_eq!(reduction.candidate, remote);
    assert_eq!(resolutions(&reduction), vec![Resolution::Committed]);
    assert_eq!(reduction.outcome, ReductionOutcome::Converged);
}

#[test]
fn a_removal_only_drops_the_track_it_names() {
    let remote = ids(&["a", "b", "c"]);
    let reduction = reduce(&remote, &[remove(1, "b")], 0);

    assert_eq!(reduction.candidate, ids(&["a", "c"]));
    assert_eq!(resolutions(&reduction), vec![Resolution::Pending]);
    assert_eq!(reduction.outcome, ReductionOutcome::Rebased);
}

#[test]
fn an_addition_cancelled_by_a_later_removal_leaves_no_pending_intent() {
    let remote = ids(&["a", "b"]);
    let reduction = reduce(&remote, &[add_back(1, "x"), remove(2, "x")], 0);

    assert_eq!(reduction.candidate, remote);
    assert_eq!(
        resolutions(&reduction),
        vec![Resolution::Committed, Resolution::Committed]
    );
    assert_eq!(reduction.outcome, ReductionOutcome::Converged);
    assert_eq!(reduction.committed_through_sequence, 2);
}

#[test]
fn a_removal_and_re_addition_that_moves_a_track_stays_pending() {
    let remote = ids(&["a", "x", "b"]);
    let reduction = reduce(&remote, &[remove(1, "x"), add_back(2, "x")], 0);

    assert_eq!(reduction.candidate, ids(&["a", "b", "x"]));
    assert_eq!(
        resolutions(&reduction),
        vec![Resolution::Pending, Resolution::Pending]
    );
    assert_eq!(reduction.outcome, ReductionOutcome::Rebased);
    assert_eq!(reduction.committed_through_sequence, 0);
}

#[test]
fn a_removal_and_re_addition_that_restores_the_remote_order_converges() {
    let remote = ids(&["a", "x", "b"]);
    let reduction = reduce(&remote, &[remove(1, "x"), add_after(2, "x", "a")], 0);

    assert_eq!(reduction.candidate, remote);
    assert_eq!(
        resolutions(&reduction),
        vec![Resolution::Committed, Resolution::Committed]
    );
    assert_eq!(reduction.outcome, ReductionOutcome::Converged);
}

#[test]
fn a_repeated_addition_does_not_own_a_second_pending_intent() {
    let remote = ids(&["a"]);
    let reduction = reduce(&remote, &[add_back(1, "x"), add_back(2, "x")], 0);

    assert_eq!(reduction.candidate, ids(&["a", "x"]));
    assert_eq!(
        resolutions(&reduction),
        vec![Resolution::Pending, Resolution::Pending]
    );
    assert_eq!(reduction.outcome, ReductionOutcome::Rebased);
    assert_eq!(reduction.committed_through_sequence, 0);
}

#[test]
fn a_reorder_that_repositions_a_locally_added_track_is_never_retired_early() {
    let remote = ids(&["a", "b"]);
    let reduction = reduce(
        &remote,
        &[add_back(1, "x"), reorder(2, &["x", "a", "b"])],
        0,
    );

    assert_eq!(reduction.candidate, ids(&["x", "a", "b"]));
    assert_eq!(
        resolutions(&reduction),
        vec![Resolution::Pending, Resolution::Pending]
    );
    assert_eq!(reduction.outcome, ReductionOutcome::Rebased);
    assert_eq!(reduction.committed_through_sequence, 0);
}

#[test]
fn no_operation_is_retired_while_the_candidate_still_differs_from_the_remote() {
    let remote = ids(&["a", "b", "c"]);
    let reduction = reduce(
        &remote,
        &[remove(1, "zzz"), add_back(2, "a"), move_front(3, "c")],
        0,
    );

    assert_eq!(reduction.candidate, ids(&["c", "a", "b"]));
    assert!(
        resolutions(&reduction)
            .iter()
            .all(|resolution| *resolution == Resolution::Pending)
    );
    assert_eq!(reduction.committed_through_sequence, 0);
}

#[test]
fn a_reorder_never_drops_a_remote_addition_it_does_not_name() {
    let remote = ids(&["a", "d", "b", "c"]);
    let reduction = reduce(&remote, &[reorder(1, &["c", "b", "a"])], 0);

    assert_eq!(reduction.candidate, ids(&["c", "d", "b", "a"]));
    assert_eq!(resolutions(&reduction), vec![Resolution::Pending]);
    assert_eq!(reduction.outcome, ReductionOutcome::Rebased);
}

#[test]
fn a_reorder_ignores_tracks_the_remote_removed() {
    let remote = ids(&["a", "b"]);
    let reduction = reduce(&remote, &[reorder(1, &["b", "gone", "a"])], 0);

    assert_eq!(reduction.candidate, ids(&["b", "a"]));
    assert_eq!(reduction.outcome, ReductionOutcome::Rebased);
}

#[test]
fn a_reorder_that_matches_the_remote_order_is_committed() {
    let remote = ids(&["a", "b", "c"]);
    let reduction = reduce(&remote, &[reorder(1, &["a", "b", "c"])], 0);

    assert_eq!(reduction.candidate, remote);
    assert_eq!(resolutions(&reduction), vec![Resolution::Committed]);
    assert_eq!(reduction.outcome, ReductionOutcome::Converged);
}

#[test]
fn a_reorder_whose_tracks_all_vanished_is_an_explainable_conflict() {
    let remote = ids(&["a", "b"]);
    let reduction = reduce(&remote, &[reorder(1, &["x", "y"])], 0);

    assert_eq!(reduction.candidate, remote);
    assert_eq!(
        resolutions(&reduction),
        vec![Resolution::Conflict(REORDER_TARGETS_MISSING)]
    );
    assert_eq!(
        reduction.outcome,
        ReductionOutcome::Conflicted(REORDER_TARGETS_MISSING)
    );
    assert_eq!(reduction.committed_through_sequence, 1);
}

#[test]
fn a_move_of_a_track_the_remote_deleted_is_an_explainable_conflict() {
    let remote = ids(&["a", "b"]);
    let reduction = reduce(&remote, &[move_front(1, "x")], 0);

    assert_eq!(reduction.candidate, remote);
    assert_eq!(
        resolutions(&reduction),
        vec![Resolution::Conflict(MOVE_TARGET_MISSING)]
    );
    assert_eq!(
        reduction.outcome,
        ReductionOutcome::Conflicted(MOVE_TARGET_MISSING)
    );
}

#[test]
fn a_move_whose_anchor_vanished_conflicts_instead_of_guessing_a_position() {
    let remote = ids(&["a", "b"]);
    let reduction = reduce(&remote, &[move_after(1, "b", "gone")], 0);

    assert_eq!(reduction.candidate, remote);
    assert_eq!(
        resolutions(&reduction),
        vec![Resolution::Conflict(MOVE_ANCHOR_MISSING)]
    );
}

#[test]
fn an_addition_whose_anchor_vanished_keeps_the_track_and_reports_the_lost_anchor() {
    let remote = ids(&["a", "b"]);
    let reduction = reduce(&remote, &[add_after(1, "x", "gone")], 0);

    assert_eq!(reduction.candidate, ids(&["a", "b", "x"]));
    assert!(reduction.anchors_lost);
    assert_eq!(resolutions(&reduction), vec![Resolution::Pending]);
}

#[test]
fn a_move_the_remote_already_reflects_is_committed() {
    let remote = ids(&["a", "b", "c"]);
    let reduction = reduce(&remote, &[move_after(1, "b", "a")], 0);

    assert_eq!(reduction.candidate, remote);
    assert_eq!(resolutions(&reduction), vec![Resolution::Committed]);
    assert_eq!(reduction.outcome, ReductionOutcome::Converged);
}

#[test]
fn operations_replay_in_sequence_order_onto_the_fresh_remote() {
    let remote = ids(&["a", "b", "c", "d"]);
    let reduction = reduce(
        &remote,
        &[remove(1, "b"), add_after(2, "x", "a"), move_front(3, "d")],
        0,
    );

    assert_eq!(reduction.candidate, ids(&["d", "a", "x", "c"]));
    assert_eq!(
        resolutions(&reduction),
        vec![
            Resolution::Pending,
            Resolution::Pending,
            Resolution::Pending
        ]
    );
    assert_eq!(reduction.outcome, ReductionOutcome::Rebased);
    assert_eq!(reduction.committed_through_sequence, 0);
}

#[test]
fn the_committed_watermark_only_advances_when_the_candidate_reaches_the_remote() {
    let remote = ids(&["a", "b"]);

    let unconverged = reduce(
        &remote,
        &[add_back(4, "a"), remove(5, "x"), add_back(6, "new")],
        3,
    );
    assert_eq!(
        resolutions(&unconverged),
        vec![
            Resolution::Pending,
            Resolution::Pending,
            Resolution::Pending
        ]
    );
    assert_eq!(unconverged.committed_through_sequence, 3);

    let converged = reduce(&remote, &[add_back(4, "a"), remove(5, "x")], 3);
    assert_eq!(
        resolutions(&converged),
        vec![Resolution::Committed, Resolution::Committed]
    );
    assert_eq!(converged.committed_through_sequence, 5);
}

#[test]
fn a_conflicted_prefix_advances_the_watermark_without_retiring_live_intent() {
    let remote = ids(&["a", "b"]);
    let reduction = reduce(&remote, &[move_front(4, "gone"), add_back(5, "new")], 3);

    assert_eq!(
        resolutions(&reduction),
        vec![
            Resolution::Conflict(MOVE_TARGET_MISSING),
            Resolution::Pending
        ]
    );
    assert_eq!(reduction.committed_through_sequence, 4);
}

#[test]
fn a_conflicting_operation_does_not_block_the_operations_behind_it() {
    let remote = ids(&["a", "b"]);
    let reduction = reduce(&remote, &[move_front(1, "gone"), add_back(2, "x")], 0);

    assert_eq!(reduction.candidate, ids(&["a", "b", "x"]));
    assert_eq!(
        resolutions(&reduction),
        vec![
            Resolution::Conflict(MOVE_TARGET_MISSING),
            Resolution::Pending
        ]
    );
    assert_eq!(reduction.committed_through_sequence, 1);
}

#[test]
fn an_empty_pending_window_leaves_the_remote_untouched() {
    let remote = ids(&["a", "b"]);
    let reduction = reduce(&remote, &[], 7);

    assert_eq!(reduction.candidate, remote);
    assert!(reduction.operations.is_empty());
    assert_eq!(reduction.outcome, ReductionOutcome::Converged);
    assert_eq!(reduction.committed_through_sequence, 7);
}

#[test]
fn a_net_zero_reorder_pair_converges_without_leaving_pending_work() {
    let remote = ids(&["a", "b", "c"]);
    let reduction = reduce(
        &remote,
        &[reorder(1, &["c", "b", "a"]), reorder(2, &["a", "b", "c"])],
        0,
    );

    assert_eq!(reduction.candidate, remote);
    assert_eq!(
        resolutions(&reduction),
        vec![Resolution::Committed, Resolution::Committed]
    );
    assert_eq!(reduction.outcome, ReductionOutcome::Converged);
    assert_eq!(reduction.committed_through_sequence, 2);
}

#[test]
fn a_move_between_anchors_the_remote_swapped_is_an_explainable_conflict() {
    let remote = ids(&["b", "a", "x"]);
    let reduction = reduce(&remote, &[move_between(1, "x", "a", "b")], 0);

    assert_eq!(reduction.candidate, remote);
    assert_eq!(
        resolutions(&reduction),
        vec![Resolution::Conflict(MOVE_ANCHOR_INVERTED)]
    );
    assert_eq!(
        reduction.outcome,
        ReductionOutcome::Conflicted(MOVE_ANCHOR_INVERTED)
    );
}

#[test]
fn a_move_between_anchors_the_remote_split_still_lands_after_the_left_anchor() {
    let remote = ids(&["a", "new", "b", "x"]);
    let reduction = reduce(&remote, &[move_between(1, "x", "a", "b")], 0);

    assert_eq!(reduction.candidate, ids(&["a", "x", "new", "b"]));
    assert_eq!(resolutions(&reduction), vec![Resolution::Pending]);
}

#[test]
fn an_empty_playlist_accepts_the_first_addition() {
    let reduction = reduce(&[], &[add_back(1, "x")], 0);

    assert_eq!(reduction.candidate, ids(&["x"]));
    assert_eq!(reduction.outcome, ReductionOutcome::Rebased);
}
