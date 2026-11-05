use std::collections::HashMap;

use chrono::{DateTime, Timelike, Utc};
use redis::AsyncCommands;

use crate::modules::chat::{ChatResult, ChatState};

pub const KEY: &str = "chat:media:reconcile";
pub const START_HOUR_UTC: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    List,
    Tail,
    Recount,
    Done,
}

impl Stage {
    fn as_str(self) -> &'static str {
        match self {
            Stage::List => "list",
            Stage::Tail => "tail",
            Stage::Recount => "recount",
            Stage::Done => "done",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "list" => Some(Stage::List),
            "tail" => Some(Stage::Tail),
            "recount" => Some(Stage::Recount),
            "done" => Some(Stage::Done),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub day: String,
    pub stage: Stage,
    pub store: u16,
    pub after: String,
    pub drift: i64,
    pub lost: i64,
    pub orphans: i64,
}

impl Cursor {
    pub fn start(day: String, stores: &[u16]) -> Self {
        let first = stores.first().copied();
        Cursor {
            day,
            stage: first.map_or(Stage::Recount, |_| Stage::List),
            store: first.unwrap_or_default(),
            after: String::new(),
            drift: 0,
            lost: 0,
            orphans: 0,
        }
    }

    pub fn due(saved: Option<Cursor>, stores: &[u16], now: DateTime<Utc>) -> Option<Cursor> {
        if let Some(cursor) = saved.as_ref().filter(|cursor| cursor.stage != Stage::Done) {
            return Some(cursor.clone());
        }
        let today = now.date_naive().to_string();
        if now.hour() < START_HOUR_UTC || saved.is_some_and(|cursor| cursor.day == today) {
            return None;
        }
        Some(Cursor::start(today, stores))
    }

    pub fn skip_unknown_store(&mut self, stores: &[u16]) {
        let listing = matches!(self.stage, Stage::List | Stage::Tail);
        if listing && !stores.contains(&self.store) {
            self.next_store(stores);
        }
    }

    pub fn next_store(&mut self, stores: &[u16]) {
        self.after.clear();
        self.lost = 0;
        self.orphans = 0;
        match stores.iter().copied().find(|id| *id > self.store) {
            Some(id) => {
                self.store = id;
                self.stage = Stage::List;
            }
            None => self.stage = Stage::Recount,
        }
    }

    fn fields(&self) -> [(&'static str, String); 7] {
        [
            ("day", self.day.clone()),
            ("stage", self.stage.as_str().to_owned()),
            ("store", self.store.to_string()),
            ("after", self.after.clone()),
            ("drift", self.drift.to_string()),
            ("lost", self.lost.to_string()),
            ("orphans", self.orphans.to_string()),
        ]
    }

    fn from_fields(fields: &HashMap<String, String>) -> Option<Self> {
        Some(Cursor {
            day: fields.get("day")?.clone(),
            stage: Stage::parse(fields.get("stage")?)?,
            store: fields.get("store")?.parse().ok()?,
            after: fields.get("after").cloned().unwrap_or_default(),
            drift: fields.get("drift")?.parse().ok()?,
            lost: fields
                .get("lost")
                .and_then(|raw| raw.parse().ok())
                .unwrap_or(0),
            orphans: fields
                .get("orphans")
                .and_then(|raw| raw.parse().ok())
                .unwrap_or(0),
        })
    }
}

pub async fn load(chat: &ChatState) -> ChatResult<Option<Cursor>> {
    let mut conn = chat.redis.get().await?;
    let fields: HashMap<String, String> = conn.hgetall(KEY).await?;
    Ok(Cursor::from_fields(&fields))
}

pub async fn save(chat: &ChatState, cursor: &Cursor) -> ChatResult<()> {
    let mut conn = chat.redis.get().await?;
    let _: () = conn.hset_multiple(KEY, &cursor.fields()).await?;
    Ok(())
}
