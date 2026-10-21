use std::collections::HashMap;

use chrono::{Duration, Utc};
use uuid::Uuid;

use crate::modules::chat::model::MessageRow;
use crate::modules::chat::sync::answer::{Assembled, Inputs, assemble};
use crate::modules::chat::sync::changes::{Change, Delta, Versions};
use crate::modules::chat::sync::seats::{ConversationState, Seat};

const VERSION: u64 = 1_000;

struct Fixture {
    versions: Versions,
    deltas: Vec<Delta>,
    states: HashMap<Uuid, ConversationState>,
    known: HashMap<Uuid, i64>,
    rows: HashMap<Uuid, Vec<MessageRow>>,
    focus: Option<Uuid>,
    include_focus: bool,
    budget: usize,
}

impl Fixture {
    fn new() -> Self {
        Self {
            versions: Versions {
                version: VERSION,
                floor: 1,
                requests: Some(0),
                fresh: false,
            },
            deltas: Vec::new(),
            states: HashMap::new(),
            known: HashMap::new(),
            rows: HashMap::new(),
            focus: None,
            include_focus: false,
            budget: 256 * 1024,
        }
    }

    fn live(&mut self, kind: &str, last_seq: i64) -> Uuid {
        let id = Uuid::now_v7();
        self.states
            .insert(id, state(id, kind, last_seq, Some("active")));
        id
    }

    fn changed(&mut self, version: u64, change: Change) {
        self.deltas.push(Delta { version, change });
    }

    fn messages(&mut self, conversation: Uuid, seqs: std::ops::RangeInclusive<i64>, size: usize) {
        let rows = self.rows.entry(conversation).or_default();
        for seq in seqs {
            rows.push(row(conversation, seq, size, None));
        }
    }

    fn run(&self) -> Assembled {
        assemble(&Inputs {
            incarnation: Uuid::nil(),
            versions: self.versions,
            deltas: &self.deltas,
            states: &self.states,
            known: &self.known,
            rows: &self.rows,
            focus: self.focus,
            include_focus: self.include_focus,
            budget: self.budget,
            now: Utc::now(),
        })
    }
}

fn state(id: Uuid, kind: &str, last_seq: i64, member: Option<&str>) -> ConversationState {
    let seat = match member {
        Some("active" | "invited") => Seat::Live { floor: 0 },
        _ => Seat::Gone,
    };
    ConversationState {
        id,
        kind: kind.to_owned(),
        last_seq,
        key_epoch: 2,
        seat,
    }
}

fn row(conversation: Uuid, seq: i64, size: usize, expires: Option<Duration>) -> MessageRow {
    MessageRow {
        conversation_id: conversation,
        seq,
        client_id: Uuid::now_v7(),
        sender_user: "101".to_owned(),
        sender_device: Some(Uuid::now_v7()),
        class: "post".to_owned(),
        epoch: 2,
        target_seq: None,
        body: Some(vec![1; size]),
        created_at: Utc::now(),
        expires_at: expires.map(|offset| Utc::now() + offset),
    }
}

fn seqs(answer: &Assembled, conversation: Uuid) -> Vec<i64> {
    answer
        .answer
        .convs
        .iter()
        .find(|entry| entry.id == conversation)
        .map(|entry| entry.messages.iter().map(|message| message.seq).collect())
        .unwrap_or_default()
}

#[test]
fn new_messages_after_the_known_seq_come_in_order() {
    let mut fixture = Fixture::new();
    let chat = fixture.live("direct", 5);
    fixture.known.insert(chat, 2);
    fixture.messages(chat, 1..=5, 10);
    fixture.changed(VERSION, Change::Conversation(chat));

    let built = fixture.run();
    assert_eq!(built.answer.pos, VERSION);
    assert_eq!(seqs(&built, chat), vec![3, 4, 5]);
    let entry = &built.answer.convs[0];
    assert!(!entry.more);
    assert_eq!((entry.last_seq, entry.key_epoch), (5, 2));
}

#[test]
fn the_byte_budget_cuts_messages_and_flags_more() {
    let mut fixture = Fixture::new();
    let chat = fixture.live("group", 10);
    fixture.known.insert(chat, 0);
    fixture.messages(chat, 1..=10, 3_000);
    fixture.changed(VERSION, Change::Conversation(chat));
    fixture.budget = 10_000;

    let built = fixture.run();
    let delivered = seqs(&built, chat);
    assert!(!delivered.is_empty() && delivered.len() < 10);
    assert_eq!(delivered, (1..=delivered.len() as i64).collect::<Vec<_>>());
    assert!(built.answer.convs[0].more);
    assert_eq!(built.answer.pos, VERSION);
}

#[test]
fn a_conversation_the_client_never_listed_comes_as_a_header() {
    let mut fixture = Fixture::new();
    let chat = fixture.live("group", 4);
    fixture.messages(chat, 1..=4, 10);
    fixture.changed(VERSION, Change::Conversation(chat));

    let built = fixture.run();
    assert!(seqs(&built, chat).is_empty());
    assert!(built.answer.convs[0].more);
}

#[test]
fn history_hidden_for_the_viewer_is_never_sent() {
    let mut fixture = Fixture::new();
    let chat = fixture.live("group", 6);
    fixture.states.get_mut(&chat).expect("a state").seat = Seat::Live { floor: 4 };
    fixture.known.insert(chat, 1);
    fixture.messages(chat, 1..=6, 10);
    fixture.changed(VERSION, Change::Conversation(chat));

    assert_eq!(seqs(&fixture.run(), chat), vec![5, 6]);
}

#[test]
fn expired_rows_are_never_served() {
    let mut fixture = Fixture::new();
    let chat = fixture.live("direct", 3);
    fixture.known.insert(chat, 0);
    fixture.rows.insert(
        chat,
        vec![
            row(chat, 1, 10, Some(Duration::seconds(-1))),
            row(chat, 2, 10, Some(Duration::hours(1))),
            row(chat, 3, 10, None),
        ],
    );
    fixture.changed(VERSION, Change::Conversation(chat));

    assert_eq!(seqs(&fixture.run(), chat), vec![2, 3]);
}

#[test]
fn left_deleted_vanished_and_departed_room_conversations_are_gone() {
    let mut fixture = Fixture::new();
    let left = Uuid::now_v7();
    fixture
        .states
        .insert(left, state(left, "group", 3, Some("left")));
    let deleted = fixture.live("group", 3);
    fixture.states.get_mut(&deleted).expect("a state").seat = Seat::Gone;
    let vanished = Uuid::now_v7();
    let departed = Uuid::now_v7();
    fixture
        .states
        .insert(departed, state(departed, "room", 3, None));
    let room = fixture.live("room", 3);
    let invited = Uuid::now_v7();
    fixture
        .states
        .insert(invited, state(invited, "direct", 1, Some("invited")));
    for (index, id) in [left, deleted, vanished, departed, room, invited]
        .into_iter()
        .enumerate()
    {
        fixture.changed(VERSION - 10 + index as u64, Change::Conversation(id));
    }

    let built = fixture.run();
    assert_eq!(built.answer.gone, vec![left, deleted, vanished, departed]);
    let listed: Vec<Uuid> = built.answer.convs.iter().map(|entry| entry.id).collect();
    assert_eq!(listed, vec![room, invited]);
    assert_eq!(built.answer.pos, VERSION);
}

#[test]
fn gone_messages_devices_and_requests_are_passed_through() {
    let mut fixture = Fixture::new();
    let chat = Uuid::now_v7();
    fixture.changed(VERSION - 2, Change::GoneMessages(chat, vec![3, 4]));
    fixture.changed(VERSION - 1, Change::Devices);
    fixture.changed(VERSION, Change::Requests);

    let built = fixture.run();
    assert_eq!(built.answer.gone_msgs, vec![(chat, 3), (chat, 4)]);
    assert!(built.wants_devices);
    assert!(built.wants_requests);
}

#[test]
fn a_spent_budget_stops_between_versions_and_keeps_the_rest_for_the_next_poll() {
    let mut fixture = Fixture::new();
    fixture.budget = 2_000;
    let mut ids = Vec::new();
    for index in 0..40_u64 {
        let id = fixture.live("group", 1);
        let version = VERSION - 40 + index / 2;
        fixture.changed(version, Change::Conversation(id));
        ids.push((version, id));
    }

    let built = fixture.run();
    let listed = built.answer.convs.len();
    assert!(listed > 0 && listed < 40);
    assert_eq!(listed % 2, 0, "a version is never split");
    assert_eq!(built.answer.pos, ids[listed - 1].0);
    assert!(built.answer.pos < VERSION);
}

#[test]
fn even_an_empty_budget_moves_forward_by_one_version() {
    let mut fixture = Fixture::new();
    fixture.budget = 0;
    let first = fixture.live("group", 1);
    let second = fixture.live("group", 1);
    fixture.changed(VERSION - 1, Change::Conversation(first));
    fixture.changed(VERSION, Change::Conversation(second));

    let built = fixture.run();
    assert_eq!(built.answer.convs.len(), 1);
    assert_eq!(built.answer.pos, VERSION - 1);
}

#[test]
fn the_focused_conversation_leads_the_answer_once() {
    let mut fixture = Fixture::new();
    let other = fixture.live("group", 1);
    let focus = fixture.live("group", 2);
    fixture.focus = Some(focus);
    fixture.include_focus = true;
    fixture.changed(VERSION - 1, Change::Conversation(other));
    fixture.changed(VERSION, Change::Conversation(focus));

    let built = fixture.run();
    assert_eq!(built.focus_live, Some(true));
    let listed: Vec<Uuid> = built.answer.convs.iter().map(|entry| entry.id).collect();
    assert_eq!(listed, vec![focus, other]);
}

#[test]
fn a_focus_on_a_conversation_the_viewer_left_is_reported_gone() {
    let mut fixture = Fixture::new();
    let focus = Uuid::now_v7();
    fixture
        .states
        .insert(focus, state(focus, "group", 2, Some("kicked")));
    fixture.focus = Some(focus);
    fixture.include_focus = true;

    let built = fixture.run();
    assert_eq!(built.focus_live, Some(false));
    assert_eq!(built.answer.gone, vec![focus]);
    assert_eq!(built.answer.pos, VERSION);
}

#[test]
fn a_change_to_the_focused_chat_rechecks_the_focus_without_a_focus_wake() {
    let mut fixture = Fixture::new();
    let focus = Uuid::now_v7();
    fixture
        .states
        .insert(focus, state(focus, "group", 2, Some("kicked")));
    fixture.focus = Some(focus);
    fixture.changed(VERSION, Change::Conversation(focus));

    let built = fixture.run();
    assert_eq!(built.focus_live, Some(false));
    assert_eq!(built.answer.gone, vec![focus]);
}

#[test]
fn an_unrelated_change_leaves_the_focus_unchecked() {
    let mut fixture = Fixture::new();
    let other = fixture.live("group", 1);
    fixture.focus = Some(Uuid::now_v7());
    fixture.changed(VERSION, Change::Conversation(other));

    assert_eq!(fixture.run().focus_live, None);
}
