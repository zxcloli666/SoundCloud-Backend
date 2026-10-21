use chat_wire::msg::Class;
use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::chat::ChatResult;
use crate::modules::chat::conversations::create::{self, DirectPlan};
use crate::modules::chat::members::access::{self, Access};
use crate::modules::chat::messages::rules::{self, Outgoing};
use crate::modules::chat::messages::store;
use crate::modules::chat::model::{MemberState, MessageRow};

pub const DEVICE: Uuid = Uuid::from_u128(0xd0);

pub async fn dm(pg: &PgPool, user: &str, peer: &str, peer_state: MemberState) -> Uuid {
    let plan = DirectPlan::New {
        peer: peer.to_owned(),
        peer_state,
    };
    create::direct(pg, user, plan).await.expect("a dm").id
}

pub async fn group(pg: &PgPool, owner: &str, members: &[(&str, MemberState)]) -> Uuid {
    let placed = members
        .iter()
        .map(|(user, state)| ((*user).to_owned(), *state))
        .collect();
    create::group(pg, owner, placed, None)
        .await
        .expect("a group")
        .id
}

pub async fn access_of(pg: &PgPool, conversation: Uuid, user: &str) -> Access {
    access::load(pg, conversation, user)
        .await
        .expect("a member")
}

pub fn outgoing(class: Class, epoch: i32, target_seq: Option<i64>) -> Outgoing {
    Outgoing {
        client_id: Uuid::now_v7(),
        class,
        epoch,
        target_seq,
        body: vec![0x5a; 256],
    }
}

pub async fn send(
    pg: &PgPool,
    conversation: Uuid,
    user: &str,
    out: &Outgoing,
) -> ChatResult<MessageRow> {
    let access = access::load(pg, conversation, user).await?;
    rules::may_post(&access, out.class)?;
    let target = store::visible_target(pg, &access, out.target_seq).await?;
    let inherited = rules::inherited_expiry(out.class, user, target.as_ref(), chrono::Utc::now())?;
    store::store(pg, &access, DEVICE, out, inherited).await
}

pub async fn post(pg: &PgPool, conversation: Uuid, user: &str) -> MessageRow {
    let epoch = access_of(pg, conversation, user).await.key_epoch;
    send(pg, conversation, user, &outgoing(Class::Post, epoch, None))
        .await
        .expect("a post")
}

pub async fn follow(pg: &PgPool, follower: &str, target: &str) {
    sqlx::query("INSERT INTO user_followings (user_id, target_user_urn) VALUES ($1, $2)")
        .bind(follower)
        .bind(format!("soundcloud:users:{target}"))
        .execute(pg)
        .await
        .expect("a follow");
}

pub async fn dm_policy(pg: &PgPool, user: &str, policy: &str) {
    sqlx::query("INSERT INTO chat.users (user_id, dm_policy) VALUES ($1, $2)")
        .bind(user)
        .bind(policy)
        .execute(pg)
        .await
        .expect("a policy");
}

pub async fn exec(pg: &PgPool, sql: &str, conversation: Uuid) {
    sqlx::query(sql)
        .bind(conversation)
        .execute(pg)
        .await
        .expect("a fixture update");
}

pub async fn seqs(pg: &PgPool, conversation: Uuid) -> Vec<i64> {
    sqlx::query_scalar("SELECT seq FROM chat.messages WHERE conversation_id = $1 ORDER BY seq")
        .bind(conversation)
        .fetch_all(pg)
        .await
        .expect("seqs")
}
