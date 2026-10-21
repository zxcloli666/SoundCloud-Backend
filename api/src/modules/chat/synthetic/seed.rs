use chat_wire::bind::make_bind;
use chat_wire::prost::Message;
use chrono::Utc;
use serde::Serialize;
use uuid::Uuid;

use crate::modules::chat::conversations::keys::dm_key;
use crate::modules::chat::members::roles::role_name;
use crate::modules::chat::model::{ConversationKind, MemberRole};
use crate::modules::chat::synthetic::Seed;
use crate::modules::chat::synthetic::layout;
use crate::modules::chat::synthetic::sessions::{self, NewSession};
use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub const ROWS_PER_INSERT: usize = 5_000;
pub const KEYS_PER_YIELD: usize = 256;

#[derive(Serialize)]
pub struct Seeded {
    pub first: u64,
    pub devices: Vec<SeededDevice>,
    pub conversations: Vec<SeededConversation>,
}

#[derive(Serialize)]
pub struct SeededDevice {
    pub user: String,
    pub id: Uuid,
    pub session: Uuid,
    #[serde(with = "crate::modules::chat::b64")]
    pub auth_seed: Vec<u8>,
    #[serde(with = "crate::modules::chat::b64")]
    pub box_secret: Vec<u8>,
    #[serde(skip)]
    pub auth_pub: Vec<u8>,
    #[serde(skip)]
    pub box_pub: Vec<u8>,
    #[serde(skip)]
    pub bind: Vec<u8>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SeededConversation {
    pub id: Uuid,
    pub kind: ConversationKind,
    pub members: Vec<String>,
}

pub async fn run(chat: &ChatState, seed: &Seed) -> ChatResult<Seeded> {
    layout::check(seed)?;
    let first = last_index(chat).await? + 1;
    let users: Vec<String> = (first..first + u64::from(seed.users))
        .map(layout::user_id)
        .collect();
    let devices = devices(&users, seed.devices_per_user).await?;
    let conversations = conversations(&users, seed.groups as usize);
    insert_users(chat, &users).await?;
    insert_devices(chat, &devices).await?;
    insert_conversations(chat, &conversations).await?;
    insert_members(chat, &conversations).await?;
    store_sessions(chat, &devices).await?;
    Ok(Seeded {
        first,
        devices,
        conversations,
    })
}

async fn last_index(chat: &ChatState) -> ChatResult<u64> {
    let last = sqlx::query_file_scalar!("queries/chat/synthetic/last_user.sql")
        .fetch_one(&chat.pg)
        .await?;
    u64::try_from(last).map_err(|_| ChatError::Internal)
}

async fn devices(users: &[String], per_user: u32) -> ChatResult<Vec<SeededDevice>> {
    let ts_ms = u64::try_from(Utc::now().timestamp_millis()).unwrap_or(0);
    let mut made = Vec::with_capacity(users.len() * per_user as usize);
    for user in users {
        for _ in 0..per_user {
            made.push(device(user, ts_ms)?);
            if made.len() % KEYS_PER_YIELD == 0 {
                tokio::task::yield_now().await;
            }
        }
    }
    Ok(made)
}

fn device(user: &str, ts_ms: u64) -> ChatResult<SeededDevice> {
    let auth_seed: [u8; 32] = chat_wire::random_bytes().map_err(|_| ChatError::Internal)?;
    let box_secret: [u8; 32] = chat_wire::random_bytes().map_err(|_| ChatError::Internal)?;
    let box_pub = chat_wire::ck::box_public(&box_secret).map_err(|_| ChatError::Internal)?;
    let id = Uuid::now_v7();
    let bind = make_bind(&id.to_string(), &auth_seed, &box_pub, ts_ms);
    Ok(SeededDevice {
        user: user.to_owned(),
        id,
        session: Uuid::new_v4(),
        auth_seed: auth_seed.to_vec(),
        box_secret: box_secret.to_vec(),
        auth_pub: bind.auth_pub.clone(),
        box_pub: box_pub.to_vec(),
        bind: bind.encode_to_vec(),
    })
}

fn conversations(users: &[String], groups: usize) -> Vec<SeededConversation> {
    let sizes = layout::group_sizes(users.len(), groups, &mut rand::thread_rng());
    layout::split(users, &sizes)
        .into_iter()
        .map(|members| SeededConversation {
            id: Uuid::now_v7(),
            kind: match members.len() {
                2 => ConversationKind::Direct,
                _ => ConversationKind::Group,
            },
            members: members.to_vec(),
        })
        .collect()
}

async fn insert_users(chat: &ChatState, users: &[String]) -> ChatResult<()> {
    for batch in users.chunks(ROWS_PER_INSERT) {
        sqlx::query_file!("queries/chat/synthetic/insert_users.sql", batch)
            .execute(&chat.pg)
            .await?;
    }
    Ok(())
}

async fn insert_devices(chat: &ChatState, devices: &[SeededDevice]) -> ChatResult<()> {
    for batch in devices.chunks(ROWS_PER_INSERT) {
        let ids: Vec<Uuid> = batch.iter().map(|device| device.id).collect();
        let users: Vec<String> = batch.iter().map(|device| device.user.clone()).collect();
        let sessions: Vec<Uuid> = batch.iter().map(|device| device.session).collect();
        let auth: Vec<Vec<u8>> = batch.iter().map(|device| device.auth_pub.clone()).collect();
        let boxes: Vec<Vec<u8>> = batch.iter().map(|device| device.box_pub.clone()).collect();
        sqlx::query_file!(
            "queries/chat/synthetic/insert_devices.sql",
            &ids,
            &users,
            &sessions,
            &auth,
            &boxes
        )
        .execute(&chat.pg)
        .await?;
    }
    Ok(())
}

async fn insert_conversations(
    chat: &ChatState,
    conversations: &[SeededConversation],
) -> ChatResult<()> {
    let (directs, groups): (Vec<&SeededConversation>, Vec<&SeededConversation>) = conversations
        .iter()
        .partition(|conversation| conversation.kind == ConversationKind::Direct);
    for batch in directs.chunks(ROWS_PER_INSERT) {
        let ids: Vec<Uuid> = batch.iter().map(|direct| direct.id).collect();
        let keys: Vec<String> = batch
            .iter()
            .map(|direct| dm_key(&direct.members[0], &direct.members[1]))
            .collect();
        let creators: Vec<String> = batch
            .iter()
            .map(|direct| direct.members[0].clone())
            .collect();
        sqlx::query_file!(
            "queries/chat/synthetic/insert_directs.sql",
            &ids,
            &keys,
            &creators
        )
        .execute(&chat.pg)
        .await?;
    }
    for batch in groups.chunks(ROWS_PER_INSERT) {
        let ids: Vec<Uuid> = batch.iter().map(|group| group.id).collect();
        let creators: Vec<String> = batch.iter().map(|group| group.members[0].clone()).collect();
        let counts: Vec<i32> = batch
            .iter()
            .map(|group| i32::try_from(group.members.len()).unwrap_or(i32::MAX))
            .collect();
        sqlx::query_file!(
            "queries/chat/synthetic/insert_groups.sql",
            &ids,
            &creators,
            &counts
        )
        .execute(&chat.pg)
        .await?;
    }
    Ok(())
}

async fn insert_members(chat: &ChatState, conversations: &[SeededConversation]) -> ChatResult<()> {
    let rows: Vec<(Uuid, &str, &'static str)> = conversations
        .iter()
        .flat_map(|conversation| {
            conversation
                .members
                .iter()
                .enumerate()
                .map(|(at, member)| (conversation.id, member.as_str(), role(conversation, at)))
        })
        .collect();
    for batch in rows.chunks(ROWS_PER_INSERT) {
        let ids: Vec<Uuid> = batch.iter().map(|row| row.0).collect();
        let users: Vec<String> = batch.iter().map(|row| row.1.to_owned()).collect();
        let roles: Vec<String> = batch.iter().map(|row| row.2.to_owned()).collect();
        sqlx::query_file!(
            "queries/chat/synthetic/insert_members.sql",
            &ids,
            &users,
            &roles
        )
        .execute(&chat.pg)
        .await?;
    }
    Ok(())
}

fn role(conversation: &SeededConversation, at: usize) -> &'static str {
    match (conversation.kind, at) {
        (ConversationKind::Group, 0) => role_name(MemberRole::Owner),
        _ => role_name(MemberRole::Member),
    }
}

async fn store_sessions(chat: &ChatState, devices: &[SeededDevice]) -> ChatResult<()> {
    let sessions: Vec<NewSession<'_>> = devices
        .iter()
        .map(|device| NewSession {
            id: device.session,
            user_id: &device.user,
            bind: device.bind.clone(),
        })
        .collect();
    sessions::store(chat, &sessions).await
}
