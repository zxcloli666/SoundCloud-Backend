use std::collections::HashSet;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::common::sc_ids::normalize_sc_track_id;
use crate::error::{AppError, AppResult};

pub const MAX_DERIVED_OPERATIONS: usize = 4_096;
pub const MAX_SUBMITTED_TRACKS: usize = 20_000;

#[derive(Debug, Clone, Deserialize)]
pub struct MoveBody {
    pub track: String,
    pub to: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditBody {
    #[serde(default)]
    pub add: Option<String>,
    #[serde(default)]
    pub remove: Option<String>,
    #[serde(default, rename = "move")]
    pub move_op: Option<MoveBody>,
    #[serde(default)]
    pub order: Option<Vec<String>>,
    #[serde(default, alias = "expected_projection_revision")]
    pub expected_projection_revision: Option<i64>,
}

pub enum TrackEdit {
    Add { track_id: String },
    Remove { track_id: String },
    Move { track_id: String, to_index: i64 },
    Order { track_ids: Vec<String> },
    Replace { track_ids: Vec<String> },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Boundary {
    Front,
    Back,
}

impl Boundary {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Front => "front",
            Self::Back => "back",
        }
    }
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
pub enum Operation {
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

impl Operation {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Add { .. } => "add",
            Self::Remove { .. } => "remove",
            Self::Move { .. } => "move",
            Self::Reorder { .. } => "reorder",
        }
    }

    pub fn track_id(&self) -> Option<&str> {
        match self {
            Self::Add { track_id, .. }
            | Self::Remove { track_id }
            | Self::Move { track_id, .. } => Some(track_id),
            Self::Reorder { .. } => None,
        }
    }

    pub fn left_anchor(&self) -> Option<&str> {
        match self.placement() {
            Some(Placement::Anchored { left, .. }) => left.as_deref(),
            _ => None,
        }
    }

    pub fn right_anchor(&self) -> Option<&str> {
        match self.placement() {
            Some(Placement::Anchored { right, .. }) => right.as_deref(),
            _ => None,
        }
    }

    pub fn boundary(&self) -> Option<&'static str> {
        match self.placement() {
            Some(Placement::Boundary(boundary)) => Some(boundary.as_str()),
            _ => None,
        }
    }

    pub fn ordered_track_ids(&self) -> Option<&[String]> {
        match self {
            Self::Reorder { ordered_track_ids } => Some(ordered_track_ids),
            _ => None,
        }
    }

    pub fn added_track(&self) -> Option<&str> {
        match self {
            Self::Add { track_id, .. } => Some(track_id),
            _ => None,
        }
    }

    pub fn fingerprint(&self) -> Vec<u8> {
        let mut digest = Sha256::new();
        absorb(&mut digest, self.kind().as_bytes());
        absorb_optional(&mut digest, self.track_id());
        absorb_optional(&mut digest, self.left_anchor());
        absorb_optional(&mut digest, self.right_anchor());
        absorb_optional(&mut digest, self.boundary());
        let ordered = self.ordered_track_ids().unwrap_or_default();
        digest.update((ordered.len() as u64).to_be_bytes());
        for track_id in ordered {
            absorb(&mut digest, track_id.as_bytes());
        }
        digest.finalize().to_vec()
    }

    fn placement(&self) -> Option<&Placement> {
        match self {
            Self::Add { placement, .. } | Self::Move { placement, .. } => Some(placement),
            Self::Remove { .. } | Self::Reorder { .. } => None,
        }
    }

    pub fn apply(&self, projection: &mut Vec<String>) {
        match self {
            Self::Add {
                track_id,
                placement,
            } => {
                if position_of(projection, track_id).is_none() {
                    let position = insert_position(projection, placement);
                    projection.insert(position, track_id.clone());
                }
            }
            Self::Remove { track_id } => {
                if let Some(from) = position_of(projection, track_id) {
                    projection.remove(from);
                }
            }
            Self::Move {
                track_id,
                placement,
            } => {
                if let Some(from) = position_of(projection, track_id) {
                    projection.remove(from);
                    let position = insert_position(projection, placement);
                    projection.insert(position, track_id.clone());
                }
            }
            Self::Reorder { ordered_track_ids } => {
                let members: HashSet<&str> = projection.iter().map(String::as_str).collect();
                let named: Vec<String> = ordered_track_ids
                    .iter()
                    .filter(|track_id| members.contains(track_id.as_str()))
                    .cloned()
                    .collect();
                let renamed: HashSet<&str> = named.iter().map(String::as_str).collect();
                let slots: Vec<usize> = projection
                    .iter()
                    .enumerate()
                    .filter(|(_, member)| renamed.contains(member.as_str()))
                    .map(|(position, _)| position)
                    .collect();
                for (slot, track_id) in slots.into_iter().zip(named) {
                    projection[slot] = track_id;
                }
            }
        }
    }
}

pub struct MembershipRequest {
    pub edit: TrackEdit,
    pub expected_projection_revision: Option<i64>,
}

impl EditBody {
    pub fn into_request(self) -> AppResult<MembershipRequest> {
        let expected_projection_revision = self.expected_projection_revision;
        let edit = match (self.add, self.remove, self.move_op, self.order) {
            (Some(track), None, None, None) => TrackEdit::Add {
                track_id: track_id_of(&track)?,
            },
            (None, Some(track), None, None) => TrackEdit::Remove {
                track_id: track_id_of(&track)?,
            },
            (None, None, Some(moved), None) => TrackEdit::Move {
                track_id: track_id_of(&moved.track)?,
                to_index: moved.to,
            },
            (None, None, None, Some(order)) => TrackEdit::Order {
                track_ids: track_ids_of(&order)?,
            },
            _ => {
                return Err(AppError::bad_request(
                    "provide exactly one of add|remove|move|order",
                ));
            }
        };
        Ok(MembershipRequest {
            edit,
            expected_projection_revision,
        })
    }
}

pub fn derive(edit: &TrackEdit, projection: &[String]) -> AppResult<Vec<Operation>> {
    let operations = match edit {
        TrackEdit::Add { track_id } => {
            if contains(projection, track_id) {
                Vec::new()
            } else {
                vec![Operation::Add {
                    track_id: track_id.clone(),
                    placement: Placement::Boundary(Boundary::Back),
                }]
            }
        }
        TrackEdit::Remove { track_id } => {
            if contains(projection, track_id) {
                vec![Operation::Remove {
                    track_id: track_id.clone(),
                }]
            } else {
                Vec::new()
            }
        }
        TrackEdit::Move { track_id, to_index } => derive_move(track_id, *to_index, projection),
        TrackEdit::Order { track_ids } => derive_order(track_ids, projection, false),
        TrackEdit::Replace { track_ids } => derive_order(track_ids, projection, true),
    };
    if operations.len() > MAX_DERIVED_OPERATIONS {
        return Err(AppError::bad_request(
            "playlist edit derives too many operations",
        ));
    }
    Ok(operations)
}

fn derive_move(track_id: &str, to_index: i64, projection: &[String]) -> Vec<Operation> {
    let Some(from) = position_of(projection, track_id) else {
        return Vec::new();
    };
    let mut without: Vec<&String> = projection.iter().collect();
    without.remove(from);
    let target = to_index.max(0).min(without.len() as i64) as usize;
    if target == from {
        return Vec::new();
    }
    let placement = if target == 0 {
        Placement::Boundary(Boundary::Front)
    } else if target == without.len() {
        Placement::Boundary(Boundary::Back)
    } else {
        Placement::Anchored {
            left: without.get(target - 1).map(|value| (*value).clone()),
            right: without.get(target).map(|value| (*value).clone()),
        }
    };
    vec![Operation::Move {
        track_id: track_id.to_owned(),
        placement,
    }]
}

fn derive_order(submitted: &[String], projection: &[String], replace: bool) -> Vec<Operation> {
    let members: HashSet<&str> = projection.iter().map(String::as_str).collect();
    let named: HashSet<&str> = submitted.iter().map(String::as_str).collect();
    let mut operations = Vec::new();
    let mut previous: Option<String> = None;
    for track_id in submitted {
        if !members.contains(track_id.as_str()) {
            operations.push(Operation::Add {
                track_id: track_id.clone(),
                placement: match &previous {
                    Some(left) => Placement::Anchored {
                        left: Some(left.clone()),
                        right: None,
                    },
                    None => Placement::Boundary(Boundary::Front),
                },
            });
        }
        previous = Some(track_id.clone());
    }
    if replace {
        for track_id in projection {
            if !named.contains(track_id.as_str()) {
                operations.push(Operation::Remove {
                    track_id: track_id.clone(),
                });
            }
        }
    }
    let mut projected = projection.to_vec();
    for operation in &operations {
        operation.apply(&mut projected);
    }
    let reorder = Operation::Reorder {
        ordered_track_ids: submitted.to_vec(),
    };
    let mut reordered = projected.clone();
    reorder.apply(&mut reordered);
    if reordered != projected {
        operations.push(reorder);
    }
    operations
}

fn insert_position(projection: &[String], placement: &Placement) -> usize {
    match placement {
        Placement::Boundary(Boundary::Front) => 0,
        Placement::Boundary(Boundary::Back) => projection.len(),
        Placement::Anchored { left, right } => left
            .as_deref()
            .and_then(|anchor| position_of(projection, anchor))
            .map(|position| position + 1)
            .or_else(|| {
                right
                    .as_deref()
                    .and_then(|anchor| position_of(projection, anchor))
            })
            .unwrap_or(projection.len()),
    }
}

pub fn track_id_of(value: &str) -> AppResult<String> {
    normalize_sc_track_id(value)
        .ok_or_else(|| AppError::bad_request("track identifier is not a SoundCloud track"))
}

pub fn track_ids_of(values: &[String]) -> AppResult<Vec<String>> {
    if values.len() > MAX_SUBMITTED_TRACKS {
        return Err(AppError::bad_request(
            "playlist edit carries too many tracks",
        ));
    }
    let mut seen: HashSet<String> = HashSet::with_capacity(values.len());
    let mut track_ids: Vec<String> = Vec::with_capacity(values.len());
    for value in values {
        let track_id = track_id_of(value)?;
        if seen.insert(track_id.clone()) {
            track_ids.push(track_id);
        }
    }
    Ok(track_ids)
}

fn absorb(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value);
}

fn absorb_optional(digest: &mut Sha256, value: Option<&str>) {
    match value {
        Some(value) => {
            digest.update([1u8]);
            absorb(digest, value.as_bytes());
        }
        None => digest.update([0u8]),
    }
}

fn position_of(projection: &[String], track_id: &str) -> Option<usize> {
    projection.iter().position(|member| member == track_id)
}

fn contains(projection: &[String], track_id: &str) -> bool {
    projection.iter().any(|member| member == track_id)
}
