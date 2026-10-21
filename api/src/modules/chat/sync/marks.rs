use std::collections::{BTreeMap, HashMap};
use std::sync::LazyLock;

use redis::Script;
use uuid::Uuid;

use crate::modules::chat::sync::wire::{PeerMark, SyncRequest};
use crate::modules::chat::sync::{bump, prefs};
use crate::modules::chat::{ChatResult, ChatState, rt_keys};

static MARKS: LazyLock<Script> = LazyLock::new(|| Script::new(include_str!("marks.lua")));

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mark {
    Read,
    Delivered,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wanted {
    pub conversation: Uuid,
    pub mark: Mark,
    pub seq: i64,
}

struct Target {
    last_seq: i64,
    peer: Option<String>,
}

pub fn wanted(request: &SyncRequest) -> Vec<Wanted> {
    let mut best: BTreeMap<(Uuid, Mark), i64> = BTreeMap::new();
    let entries = request.read.iter().map(|entry| (entry, Mark::Read)).chain(
        request
            .delivered
            .iter()
            .map(|entry| (entry, Mark::Delivered)),
    );
    for (&(conversation, seq), mark) in entries {
        if seq > 0 {
            let kept = best.entry((conversation, mark)).or_insert(seq);
            *kept = (*kept).max(seq);
        }
    }
    best.into_iter()
        .map(|((conversation, mark), seq)| Wanted {
            conversation,
            mark,
            seq,
        })
        .collect()
}

pub async fn record(chat: &ChatState, user_id: &str, request: &SyncRequest) -> ChatResult<()> {
    let wanted = wanted(request);
    if wanted.is_empty() {
        return Ok(());
    }
    let mut ids: Vec<Uuid> = wanted.iter().map(|entry| entry.conversation).collect();
    ids.dedup();
    let targets: HashMap<Uuid, Target> =
        sqlx::query_file!("queries/chat/sync/mark_targets.sql", &ids, user_id)
            .fetch_all(&chat.pg)
            .await?
            .into_iter()
            .map(|row| {
                (
                    row.id,
                    Target {
                        last_seq: row.last_seq,
                        peer: row.peer,
                    },
                )
            })
            .collect();
    let marks: Vec<Wanted> = wanted
        .into_iter()
        .filter_map(|entry| {
            let target = targets.get(&entry.conversation)?;
            let seq = entry.seq.min(target.last_seq);
            (seq > 0).then_some(Wanted { seq, ..entry })
        })
        .collect();
    if marks.is_empty() {
        return Ok(());
    }
    let mut call = MARKS.prepare_invoke();
    for entry in &marks {
        call.key(match entry.mark {
            Mark::Read => rt_keys::read_marks(entry.conversation),
            Mark::Delivered => rt_keys::delivered_marks(entry.conversation),
        });
    }
    call.arg(rt_keys::MARKS_TTL.as_secs()).arg(user_id);
    for entry in &marks {
        call.arg(entry.seq);
    }
    let changed: Vec<usize> = {
        let mut conn = chat.redis.get().await?;
        call.invoke_async(&mut conn).await?
    };
    let moved: Vec<&Wanted> = changed
        .iter()
        .filter_map(|index| marks.get(index.checked_sub(1)?))
        .collect();
    for (conversation, kinds) in moved
        .chunk_by(|left, right| left.conversation == right.conversation)
        .map(|group| (group[0].conversation, group))
    {
        let peer = targets
            .get(&conversation)
            .and_then(|target| target.peer.clone());
        if peer.is_none() {
            chat.hub.wake_focus(conversation, user_id);
        }
        let read = kinds.iter().any(|entry| entry.mark == Mark::Read);
        let recipients: Vec<String> = peer
            .into_iter()
            .chain(read.then(|| user_id.to_owned()))
            .collect();
        bump::conversation(chat, conversation, 0, &recipients).await?;
    }
    Ok(())
}

pub async fn own(
    chat: &ChatState,
    viewer: &str,
    conversations: &[Uuid],
) -> ChatResult<HashMap<Uuid, PeerMark>> {
    if conversations.is_empty() {
        return Ok(HashMap::new());
    }
    let mut pipe = redis::pipe();
    for conversation in conversations {
        pipe.hget(rt_keys::read_marks(*conversation), viewer);
        pipe.hget(rt_keys::delivered_marks(*conversation), viewer);
    }
    let replies: Vec<Option<i64>> = {
        let mut conn = chat.redis.get().await?;
        pipe.query_async(&mut conn).await?
    };
    let (pairs, _) = replies.as_chunks::<2>();
    Ok(conversations
        .iter()
        .zip(pairs)
        .filter_map(|(conversation, [read, delivered])| {
            let read_seq = read.unwrap_or(0);
            let delivered_seq = delivered.unwrap_or(0).max(read_seq);
            (delivered_seq > 0).then(|| {
                (
                    *conversation,
                    PeerMark {
                        user_id: viewer.to_owned(),
                        read_seq,
                        delivered_seq,
                    },
                )
            })
        })
        .collect())
}

pub async fn read(
    chat: &ChatState,
    viewer: &str,
    conversations: &[Uuid],
) -> ChatResult<HashMap<Uuid, Vec<PeerMark>>> {
    if conversations.is_empty() {
        return Ok(HashMap::new());
    }
    let mut pipe = redis::pipe();
    for conversation in conversations {
        pipe.hgetall(rt_keys::read_marks(*conversation));
        pipe.hgetall(rt_keys::delivered_marks(*conversation));
    }
    let replies: Vec<HashMap<String, i64>> = {
        let mut conn = chat.redis.get().await?;
        pipe.query_async(&mut conn).await?
    };
    let (pairs, _) = replies.as_chunks::<2>();
    let mut marks: HashMap<Uuid, Vec<PeerMark>> = conversations
        .iter()
        .zip(pairs)
        .map(|(conversation, [read, delivered])| (*conversation, merge(read, delivered)))
        .collect();
    let mut users: Vec<String> = marks
        .values()
        .flatten()
        .filter(|mark| mark.user_id != viewer && mark.read_seq > 0)
        .map(|mark| mark.user_id.clone())
        .collect();
    users.sort();
    users.dedup();
    let prefs = prefs::load(chat, &users).await?;
    for mark in marks.values_mut().flatten() {
        let quiet = prefs
            .get(&mark.user_id)
            .is_some_and(|prefs| !prefs.read_receipts);
        if quiet && mark.user_id != viewer {
            mark.read_seq = 0;
        }
    }
    Ok(marks)
}

pub fn merge(read: &HashMap<String, i64>, delivered: &HashMap<String, i64>) -> Vec<PeerMark> {
    let mut users: BTreeMap<&str, (i64, i64)> = BTreeMap::new();
    for (user, seq) in read {
        users.entry(user).or_default().0 = *seq;
    }
    for (user, seq) in delivered {
        users.entry(user).or_default().1 = *seq;
    }
    users
        .into_iter()
        .map(|(user_id, (read_seq, delivered_seq))| PeerMark {
            user_id: user_id.to_owned(),
            read_seq,
            delivered_seq: delivered_seq.max(read_seq),
        })
        .collect()
}
