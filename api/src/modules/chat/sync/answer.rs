use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::modules::chat::model::{MessageRow, MessageView};
use crate::modules::chat::sync::changes::{Change, Changes, Delta, Versions};
use crate::modules::chat::sync::rows::{self, Fetch};
use crate::modules::chat::sync::seats::{self, ConversationState, Seat, seat_of};
use crate::modules::chat::sync::wire::{SyncAnswer, SyncConversation};
use crate::modules::chat::sync::{account, bytes, marks};
use crate::modules::chat::{ChatResult, ChatState, typing};

pub struct Ask<'a> {
    pub user_id: &'a str,
    pub device: Uuid,
    pub known: &'a [(Uuid, i64)],
    pub focus: Option<Uuid>,
    pub include_focus: bool,
    pub budget: usize,
}

pub struct Built {
    pub answer: SyncAnswer,
    pub focus_live: bool,
}

pub struct Inputs<'a> {
    pub incarnation: Uuid,
    pub versions: Versions,
    pub deltas: &'a [Delta],
    pub states: &'a HashMap<Uuid, ConversationState>,
    pub known: &'a HashMap<Uuid, i64>,
    pub rows: &'a HashMap<Uuid, Vec<MessageRow>>,
    pub focus: Option<Uuid>,
    pub include_focus: bool,
    pub budget: usize,
    pub now: DateTime<Utc>,
}

pub struct Assembled {
    pub answer: SyncAnswer,
    pub wants_devices: bool,
    pub wants_requests: bool,
    pub focus_live: bool,
}

pub async fn reset(
    chat: &ChatState,
    user_id: &str,
    device: Uuid,
    versions: &Versions,
    incarnation: Uuid,
) -> ChatResult<SyncAnswer> {
    Ok(SyncAnswer {
        pos: versions.version,
        inc: incarnation,
        reset: true,
        convs: Vec::new(),
        gone: Vec::new(),
        gone_msgs: Vec::new(),
        devices: account::devices(chat, user_id, device).await?,
        requests_count: account::requests_count(chat, user_id).await?,
    })
}

pub async fn build(
    chat: &ChatState,
    ask: &Ask<'_>,
    changes: &Changes,
    incarnation: Uuid,
) -> ChatResult<Built> {
    let mut ids: Vec<Uuid> = changes
        .deltas
        .iter()
        .filter_map(|delta| match delta.change {
            Change::Conversation(id) => Some(id),
            _ => None,
        })
        .chain(ask.focus.filter(|_| ask.include_focus))
        .collect();
    ids.sort();
    ids.dedup();
    let states = seats::load(chat, ask.user_id, &ids).await?;
    let known: HashMap<Uuid, i64> = ask.known.iter().copied().collect();
    let wanted: Vec<Fetch> = states
        .values()
        .filter_map(|state| fetch_for(state, &known))
        .collect();
    let rows = rows::fetch(chat, &wanted, ask.budget).await?;
    let mut assembled = assemble(&Inputs {
        incarnation,
        versions: changes.versions,
        deltas: &changes.deltas,
        states: &states,
        known: &known,
        rows: &rows,
        focus: ask.focus,
        include_focus: ask.include_focus,
        budget: ask.budget,
        now: Utc::now(),
    });
    decorate(chat, ask, &states, &mut assembled.answer).await?;
    if assembled.wants_devices {
        assembled.answer.devices = account::devices(chat, ask.user_id, ask.device).await?;
    }
    assembled.answer.requests_count = match changes.versions.requests {
        Some(count) if !assembled.wants_requests => count,
        _ => account::requests_count(chat, ask.user_id).await?,
    };
    Ok(Built {
        answer: assembled.answer,
        focus_live: assembled.focus_live,
    })
}

fn fetch_for(state: &ConversationState, known: &HashMap<Uuid, i64>) -> Option<Fetch> {
    let Seat::Live { floor } = state.seat() else {
        return None;
    };
    let after = (*known.get(&state.id)?).max(floor);
    (state.last_seq > after).then_some(Fetch {
        conversation: state.id,
        after,
    })
}

async fn decorate(
    chat: &ChatState,
    ask: &Ask<'_>,
    states: &HashMap<Uuid, ConversationState>,
    answer: &mut SyncAnswer,
) -> ChatResult<()> {
    let with_marks: Vec<Uuid> = answer
        .convs
        .iter()
        .map(|entry| entry.id)
        .filter(|id| Some(*id) == ask.focus || states.get(id).is_some_and(|s| s.is_direct()))
        .collect();
    let mut marks = marks::read(chat, ask.user_id, &with_marks).await?;
    for entry in &mut answer.convs {
        if let Some(found) = marks.remove(&entry.id) {
            entry.marks = found;
        }
        if Some(entry.id) == ask.focus {
            entry.typing = typing::typing_in(chat, entry.id)
                .await?
                .into_iter()
                .filter(|user| user != ask.user_id)
                .collect();
        }
    }
    Ok(())
}

pub fn assemble(inputs: &Inputs<'_>) -> Assembled {
    let mut assembly = Assembly {
        inputs,
        remaining: isize::try_from(inputs.budget).unwrap_or(isize::MAX),
        emitted: HashSet::new(),
        answer: SyncAnswer {
            pos: inputs.versions.version,
            inc: inputs.incarnation,
            reset: false,
            convs: Vec::new(),
            gone: Vec::new(),
            gone_msgs: Vec::new(),
            devices: Vec::new(),
            requests_count: 0,
        },
        wants_devices: false,
        wants_requests: false,
    };
    let focus_live = match inputs.focus.filter(|_| inputs.include_focus) {
        Some(focus) => {
            assembly.conversation(focus);
            matches!(seat_of(inputs.states, focus), Seat::Live { .. })
        }
        None => false,
    };
    let mut processed = false;
    let mut complete = true;
    for group in inputs
        .deltas
        .chunk_by(|left, right| left.version == right.version)
    {
        if processed && assembly.remaining <= 0 {
            complete = false;
            break;
        }
        for delta in group {
            assembly.delta(&delta.change);
        }
        assembly.answer.pos = group[0].version;
        processed = true;
    }
    if complete {
        assembly.answer.pos = inputs.versions.version;
    }
    Assembled {
        answer: assembly.answer,
        wants_devices: assembly.wants_devices,
        wants_requests: assembly.wants_requests,
        focus_live,
    }
}

struct Assembly<'a> {
    inputs: &'a Inputs<'a>,
    remaining: isize,
    emitted: HashSet<Uuid>,
    answer: SyncAnswer,
    wants_devices: bool,
    wants_requests: bool,
}

impl Assembly<'_> {
    fn delta(&mut self, change: &Change) {
        match change {
            Change::Conversation(id) => self.conversation(*id),
            Change::Devices => self.wants_devices = true,
            Change::Requests => self.wants_requests = true,
            Change::GoneMessages(id, seqs) => {
                for seq in seqs {
                    self.answer.gone_msgs.push((*id, *seq));
                    self.spend(bytes::GONE_MESSAGE);
                }
            }
        }
    }

    fn conversation(&mut self, id: Uuid) {
        if !self.emitted.insert(id) {
            return;
        }
        let state = self.inputs.states.get(&id);
        match seat_of(self.inputs.states, id) {
            Seat::Unknown => {}
            Seat::Gone => {
                self.answer.gone.push(id);
                self.spend(bytes::GONE);
            }
            Seat::Live { floor } => {
                if let Some(state) = state {
                    let entry = self.entry(state, floor);
                    self.answer.convs.push(entry);
                }
            }
        }
    }

    fn entry(&mut self, state: &ConversationState, floor: i64) -> SyncConversation {
        self.spend(bytes::CONVERSATION);
        let mut entry = SyncConversation {
            id: state.id,
            last_seq: state.last_seq,
            key_epoch: state.key_epoch,
            messages: Vec::new(),
            more: false,
            marks: Vec::new(),
            typing: Vec::new(),
        };
        let Some(&known) = self.inputs.known.get(&state.id) else {
            entry.more = state.last_seq > floor;
            return entry;
        };
        let after = known.max(floor);
        let mut delivered = after;
        let mut rows: Vec<&MessageRow> = self
            .inputs
            .rows
            .get(&state.id)
            .into_iter()
            .flatten()
            .filter(|row| row.seq > after && !expired(row, self.inputs.now))
            .collect();
        rows.sort_by_key(|row| row.seq);
        for row in rows {
            let view = MessageView::from(row);
            let cost = bytes::message(&view);
            if isize::try_from(cost).unwrap_or(isize::MAX) > self.remaining {
                break;
            }
            self.spend(cost);
            delivered = row.seq;
            entry.messages.push(view);
        }
        entry.last_seq = entry.last_seq.max(delivered);
        entry.more = delivered < entry.last_seq;
        entry
    }

    fn spend(&mut self, cost: usize) {
        self.remaining = self
            .remaining
            .saturating_sub(isize::try_from(cost).unwrap_or(isize::MAX));
    }
}

fn expired(row: &MessageRow, now: DateTime<Utc>) -> bool {
    row.expires_at.is_some_and(|at| at <= now)
}
