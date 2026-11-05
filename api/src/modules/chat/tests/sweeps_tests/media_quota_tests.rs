use std::collections::HashMap;

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use sqlx::PgPool;
use uuid::Uuid;

use super::support::{conversation, member, messages, state, user};
use crate::modules::chat::ChatState;
use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::sweeps::media::fixtures::{
    age, attach, commit, drift, handle, hide, ok, owner_delete, redact, report, reserve,
};
use crate::modules::chat::sweeps::media::{delete, expire, holds, recount, trim, withdrawn};
use crate::modules::chat::sweeps::{expired, purge};

pub const OPS: usize = 1_000;

pub struct World {
    pub live: bool,
    rng: StdRng,
    users: Vec<String>,
    chats: Vec<(Uuid, Vec<String>)>,
    rooms: Vec<(Uuid, String)>,
    seqs: HashMap<Uuid, i64>,
    reports: Vec<Uuid>,
}

impl World {
    fn pick<T: Clone>(&mut self, items: &[T]) -> Option<T> {
        (!items.is_empty()).then(|| items[self.rng.gen_range(0..items.len())].clone())
    }

    fn next_seq(&mut self, conversation: Uuid) -> i64 {
        let seq = self.seqs.entry(conversation).or_insert(0);
        *seq += 1;
        *seq
    }
}

async fn ids(pg: &PgPool, sql: &str) -> anyhow::Result<Vec<Uuid>> {
    Ok(sqlx::query_scalar(sql).fetch_all(pg).await?)
}

pub async fn world(pg: &PgPool) -> anyhow::Result<World> {
    let users: Vec<String> = (0..6).map(|_| user()).collect();
    let mut chats = Vec::new();
    for pair in users.chunks(2) {
        let dm = conversation(pg, "direct", &pair[0]).await?;
        for one in pair {
            member(pg, dm, one, "active").await?;
        }
        chats.push((dm, pair.to_vec()));
    }
    for _ in 0..2 {
        let group = conversation(pg, "group", &users[0]).await?;
        for one in &users {
            member(pg, group, one, "active").await?;
        }
        chats.push((group, users.clone()));
    }
    Ok(World {
        live: false,
        rng: StdRng::seed_from_u64(0x5eed),
        users,
        chats,
        rooms: Vec::new(),
        seqs: HashMap::new(),
        reports: Vec::new(),
    })
}

async fn post(pg: &PgPool, w: &mut World, conversation: Uuid, image: Uuid) -> anyhow::Result<()> {
    let seq = w.next_seq(conversation);
    let expires = w.rng.gen_bool(0.2).then_some("-1 second");
    messages(pg, conversation, seq..=seq, expires).await?;
    attach(pg, image, conversation, seq).await?;
    Ok(())
}

pub async fn op(chat: &ChatState, pg: &PgPool, w: &mut World) -> anyhow::Result<()> {
    let kinds = if w.live { 13 } else { 11 };
    match w.rng.gen_range(0..kinds) {
        0 | 1 => {
            let owner = w.pick(&w.users.clone()).expect("users");
            let origin = if w.rng.gen_bool(0.3) {
                room_of(pg, w, &owner).await?
            } else {
                w.pick(&w.chats.clone()).expect("chats").0
            };
            let upload = reserve(pg, &owner, origin, 1, w.rng.gen_bool(0.8)).await?;
            if w.live {
                for (key, length) in upload.objects() {
                    let bytes = vec![7; usize::try_from(length)?];
                    ok(chat.s3.put(1, &key, bytes).await)?;
                }
            }
            if w.rng.gen_bool(0.85) {
                commit(pg, &upload).await?;
            }
        }
        2 | 3 => {
            let uploaded = ids(pg, "SELECT id FROM chat.media WHERE kind <> 'thumb' AND status IN ('uploaded', 'attached')").await?;
            if let Some(image) = w.pick(&uploaded) {
                let origin: Uuid =
                    sqlx::query_scalar("SELECT origin_conversation FROM chat.media WHERE id = $1")
                        .bind(image)
                        .fetch_one(pg)
                        .await?;
                let from_chat = w.chats.iter().any(|(chat_id, _)| *chat_id == origin);
                let live_room = w.rooms.iter().any(|(room, _)| *room == origin);
                if live_room {
                    post(pg, w, origin, image).await?;
                } else if from_chat {
                    let target = w.pick(&w.chats.clone()).expect("chats").0;
                    post(pg, w, target, image).await?;
                }
            }
        }
        4 => {
            let posted: Vec<(Uuid, i64)> =
                sqlx::query_as("SELECT conversation_id, seq FROM chat.media_refs WHERE seq > 0")
                    .fetch_all(pg)
                    .await?;
            if let Some((conversation, seq)) = w.pick(&posted) {
                redact(pg, conversation, seq).await?;
            }
        }
        5 => {
            let (chat_id, members) = w.pick(&w.chats.clone()).expect("chats");
            let who = w.pick(&members).expect("members");
            let below = *w.seqs.get(&chat_id).unwrap_or(&0);
            hide(pg, chat_id, &who, below).await?;
        }
        6 => {
            let young = ids(pg, "SELECT id FROM chat.media WHERE kind <> 'thumb' AND status IN ('reserved', 'uploaded')").await?;
            if let Some(image) = w.pick(&young) {
                age(pg, image).await?;
            }
        }
        7 => {
            if let Some((room, _)) = w.pick(&w.rooms.clone()) {
                w.rooms.retain(|(id, _)| *id != room);
                ok(purge::shred(chat, room).await)?;
            }
        }
        8 => {
            let own = ids(pg, "SELECT id FROM chat.media WHERE kind <> 'thumb' AND status IN ('reserved', 'uploaded', 'attached', 'lost')").await?;
            if let Some(image) = w.pick(&own) {
                let window = if w.rng.gen_bool(0.5) { -1 } else { 15 };
                owner_delete(pg, image, window).await?;
            }
        }
        9 => {
            let attached = ids(
                pg,
                "SELECT id FROM chat.media WHERE kind <> 'thumb' AND status = 'attached'",
            )
            .await?;
            match w.pick(&attached) {
                Some(image) if w.rng.gen_bool(0.6) => {
                    let conversation: Uuid = sqlx::query_scalar(
                        "SELECT conversation_id FROM chat.media_refs WHERE media_id = $1 LIMIT 1",
                    )
                    .bind(image)
                    .fetch_one(pg)
                    .await?;
                    let case = report(pg, conversation, &[image]).await?;
                    w.reports.push(case);
                }
                _ => {
                    if let Some(case) = w.pick(&w.reports.clone()) {
                        handle(pg, case).await?;
                    }
                }
            }
        }
        11 => {
            let attached = ids(pg, "SELECT id FROM chat.media WHERE kind <> 'thumb' AND status IN ('uploaded', 'attached')").await?;
            if let Some(image) = w.pick(&attached) {
                let key: String =
                    sqlx::query_scalar("SELECT object_key FROM chat.media WHERE id = $1")
                        .bind(image)
                        .fetch_one(pg)
                        .await?;
                ok(chat.s3.delete(1, &key).await)?;
            }
        }
        12 => {
            ok(delete::run(chat, &mut Budget::new(2)).await)?;
        }
        _ => sweep(chat, w.rng.gen_range(0..6)).await?,
    }
    Ok(())
}

async fn room_of(pg: &PgPool, w: &mut World, host: &str) -> anyhow::Result<Uuid> {
    if let Some((room, _)) = w.rooms.iter().find(|(_, owner)| owner == host) {
        return Ok(*room);
    }
    let room = conversation(pg, "room", host).await?;
    w.rooms.push((room, host.to_owned()));
    Ok(room)
}

pub async fn sweep(chat: &ChatState, which: u32) -> anyhow::Result<()> {
    match which {
        0 => ok(expire::run(chat, &mut Budget::new(5)).await)?,
        1 => ok(trim::run(chat, &mut Budget::new(5)).await)?,
        2 => ok(withdrawn::run(chat, &mut Budget::new(5)).await)?,
        3 => ok(holds::run(chat, &mut Budget::new(5)).await)?,
        4 => ok(expired::run_once(chat).await)?,
        _ => ok(purge::run_once(chat).await)?,
    };
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_thousand_random_media_operations_leave_no_quota_drift(pg: PgPool) -> anyhow::Result<()> {
    let chat = state(pg.clone());
    let mut w = world(&pg).await?;
    for step in 0..OPS {
        op(&chat, &pg, &mut w).await?;
        if step % 100 == 99 {
            assert_eq!(drift(&pg).await?, 0, "drift after {} operations", step + 1);
        }
    }
    for which in 0..6 {
        sweep(&chat, which).await?;
    }
    assert_eq!(drift(&pg).await?, 0);
    assert_eq!(ok(recount::all(&chat).await)?, 0);
    let live: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chat.media WHERE status IN ('uploaded', 'attached')",
    )
    .fetch_one(&pg)
    .await?;
    assert!(live > 0, "the run kept some images alive");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn the_recount_repairs_a_drifted_usage_row_and_reports_the_drift(
    pg: PgPool,
) -> anyhow::Result<()> {
    let chat = state(pg.clone());
    let owner = user();
    let group = conversation(&pg, "group", &owner).await?;
    let image = reserve(&pg, &owner, group, 1, true).await?;
    commit(&pg, &image).await?;
    reserve(&pg, &owner, group, 1, true).await?;
    sqlx::query(
        "UPDATE chat.usage SET bytes = bytes + 4096, reserved = 0, files = 7 WHERE user_id = $1",
    )
    .bind(&owner)
    .execute(&pg)
    .await?;

    assert_eq!(ok(recount::all(&chat).await)?, 4_096 + image.total());
    assert_eq!(drift(&pg).await?, 0);
    assert_eq!(ok(recount::all(&chat).await)?, 0);
    Ok(())
}
