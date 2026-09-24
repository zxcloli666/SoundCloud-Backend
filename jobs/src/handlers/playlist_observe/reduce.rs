use std::collections::HashSet;

use uuid::Uuid;

pub const MOVE_TARGET_MISSING: &str = "move_target_missing";
pub const MOVE_ANCHOR_MISSING: &str = "move_anchor_missing";
pub const MOVE_ANCHOR_INVERTED: &str = "move_anchor_inverted";
pub const REORDER_TARGETS_MISSING: &str = "reorder_targets_missing";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Boundary {
    Front,
    Back,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Placement {
    Boundary(Boundary),
    Anchored {
        left: Option<String>,
        right: Option<String>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Intent {
    Add {
        track_id: String,
        placement: Placement,
    },
    Remove {
        track_id: String,
    },
    Move {
        track_id: String,
        placement: Placement,
    },
    Reorder {
        ordered_track_ids: Vec<String>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Operation {
    pub operation_id: Uuid,
    pub sequence: i64,
    pub intent: Intent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Resolution {
    Pending,
    Committed,
    Conflict(&'static str),
}

impl Resolution {
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Pending)
    }

    pub fn outcome(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Committed => "committed",
            Self::Conflict(_) => "conflict",
        }
    }

    pub fn conflict_code(self) -> Option<&'static str> {
        match self {
            Self::Conflict(code) => Some(code),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedOperation {
    pub operation_id: Uuid,
    pub sequence: i64,
    pub resolution: Resolution,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReductionOutcome {
    Converged,
    Rebased,
    Conflicted(&'static str),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reduction {
    pub candidate: Vec<String>,
    pub operations: Vec<ResolvedOperation>,
    pub outcome: ReductionOutcome,
    pub committed_through_sequence: i64,
    pub anchors_lost: bool,
}

pub fn reduce(remote: &[String], operations: &[Operation], committed_through: i64) -> Reduction {
    let mut candidate = remote.to_vec();
    let mut resolutions = vec![Resolution::Pending; operations.len()];
    let mut anchors_lost = false;

    for (index, operation) in operations.iter().enumerate() {
        match apply(&mut candidate, &operation.intent) {
            Ok(lost) => anchors_lost |= lost,
            Err(code) => resolutions[index] = Resolution::Conflict(code),
        }
    }

    let converged = candidate == remote;
    if converged {
        for resolution in &mut resolutions {
            if *resolution == Resolution::Pending {
                *resolution = Resolution::Committed;
            }
        }
    }

    let conflict = resolutions.iter().find_map(|resolution| match resolution {
        Resolution::Conflict(code) => Some(*code),
        _ => None,
    });
    let outcome = match conflict {
        Some(code) => ReductionOutcome::Conflicted(code),
        None if converged => ReductionOutcome::Converged,
        None => ReductionOutcome::Rebased,
    };

    let operations: Vec<ResolvedOperation> = operations
        .iter()
        .zip(resolutions)
        .map(|(operation, resolution)| ResolvedOperation {
            operation_id: operation.operation_id,
            sequence: operation.sequence,
            resolution,
        })
        .collect();
    let committed_through_sequence = committed_prefix(&operations, committed_through);

    Reduction {
        candidate,
        operations,
        outcome,
        committed_through_sequence,
        anchors_lost,
    }
}

pub fn committed_prefix(operations: &[ResolvedOperation], floor: i64) -> i64 {
    let mut watermark = floor;
    for operation in operations {
        if !operation.resolution.is_terminal() {
            break;
        }
        watermark = watermark.max(operation.sequence);
    }
    watermark
}

fn apply(list: &mut Vec<String>, intent: &Intent) -> Result<bool, &'static str> {
    match intent {
        Intent::Add {
            track_id,
            placement,
        } => {
            if contains(list, track_id) {
                return Ok(false);
            }
            let (position, anchors_lost) = match resolve_placement(list, placement) {
                Some(position) => (position, false),
                None => (fallback_position(list, placement), true),
            };
            list.insert(position, track_id.clone());
            Ok(anchors_lost)
        }
        Intent::Remove { track_id } => {
            if let Some(from) = position_of(list, track_id) {
                list.remove(from);
            }
            Ok(false)
        }
        Intent::Move {
            track_id,
            placement,
        } => {
            let Some(from) = position_of(list, track_id) else {
                return Err(MOVE_TARGET_MISSING);
            };
            if anchors_are_inverted(list, placement) {
                return Err(MOVE_ANCHOR_INVERTED);
            }
            if resolve_placement(list, placement).is_none() {
                return Err(MOVE_ANCHOR_MISSING);
            }
            let moved = list.remove(from);
            let position = resolve_placement(list, placement).ok_or(MOVE_ANCHOR_MISSING)?;
            list.insert(position, moved);
            Ok(false)
        }
        Intent::Reorder { ordered_track_ids } => {
            let named = present_in_order(list, ordered_track_ids);
            if named.is_empty() {
                return if ordered_track_ids.is_empty() {
                    Ok(false)
                } else {
                    Err(REORDER_TARGETS_MISSING)
                };
            }
            let renamed: HashSet<&str> = named.iter().map(String::as_str).collect();
            let slots: Vec<usize> = list
                .iter()
                .enumerate()
                .filter(|(_, member)| renamed.contains(member.as_str()))
                .map(|(position, _)| position)
                .collect();
            for (slot, track_id) in slots.into_iter().zip(named) {
                list[slot] = track_id;
            }
            Ok(false)
        }
    }
}

fn anchors_are_inverted(list: &[String], placement: &Placement) -> bool {
    let Placement::Anchored {
        left: Some(left),
        right: Some(right),
    } = placement
    else {
        return false;
    };
    match (position_of(list, left), position_of(list, right)) {
        (Some(left), Some(right)) => left > right,
        _ => false,
    }
}

fn resolve_placement(list: &[String], placement: &Placement) -> Option<usize> {
    match placement {
        Placement::Boundary(Boundary::Front) => Some(0),
        Placement::Boundary(Boundary::Back) => Some(list.len()),
        Placement::Anchored { left, right } => left
            .as_deref()
            .and_then(|anchor| position_of(list, anchor))
            .map(|position| position + 1)
            .or_else(|| {
                right
                    .as_deref()
                    .and_then(|anchor| position_of(list, anchor))
            }),
    }
}

fn fallback_position(list: &[String], placement: &Placement) -> usize {
    match placement {
        Placement::Anchored { left: Some(_), .. } => list.len(),
        Placement::Anchored { .. } => 0,
        Placement::Boundary(Boundary::Front) => 0,
        Placement::Boundary(Boundary::Back) => list.len(),
    }
}

fn present_in_order(list: &[String], ordered_track_ids: &[String]) -> Vec<String> {
    let members: HashSet<&str> = list.iter().map(String::as_str).collect();
    let mut taken: HashSet<&str> = HashSet::new();
    let mut named: Vec<String> = Vec::new();
    for track_id in ordered_track_ids {
        if members.contains(track_id.as_str()) && taken.insert(track_id.as_str()) {
            named.push(track_id.clone());
        }
    }
    named
}

fn position_of(list: &[String], track_id: &str) -> Option<usize> {
    list.iter().position(|member| member == track_id)
}

fn contains(list: &[String], track_id: &str) -> bool {
    list.iter().any(|member| member == track_id)
}
