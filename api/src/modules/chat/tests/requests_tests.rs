use axum::extract::State;
use chrono::{Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::{
    Landing, MESSAGES_BEFORE_ACCEPT, Relation, check_send, count, evaluate, landing, landing_for,
    list, relation, session_is_young,
};
use crate::modules::chat::moderation::testing::{
    TestKeys, conversation, conversation_key, post, state,
};
use crate::modules::chat::settings::DmPolicy;
use crate::modules::chat::test_support::caller;
use crate::modules::chat::users::{self, Scope};
use crate::modules::chat::{ChatCaller, ChatError};

#[test]
fn the_dm_policy_decides_where_a_first_message_lands() {
    let stranger = Relation {
        contact: false,
        follows: false,
    };
    let followed = Relation {
        contact: false,
        follows: true,
    };
    let contact = Relation {
        contact: true,
        follows: false,
    };
    assert_eq!(landing_for(DmPolicy::Everyone, stranger), Landing::Request);
    assert_eq!(landing_for(DmPolicy::Everyone, followed), Landing::Direct);
    assert_eq!(
        landing_for(DmPolicy::Following, stranger),
        Landing::Unavailable
    );
    assert_eq!(landing_for(DmPolicy::Following, followed), Landing::Direct);
    assert_eq!(
        landing_for(DmPolicy::Nobody, followed),
        Landing::Unavailable
    );
    assert_eq!(landing_for(DmPolicy::Nobody, contact), Landing::Direct);
}

async fn open_dms(pool: &PgPool, user: &str) {
    sqlx::query("INSERT INTO chat.users (user_id, dm_policy) VALUES ($1, 'everyone')")
        .bind(user)
        .execute(pool)
        .await
        .expect("open dms");
}

#[sqlx::test(migrations = "./migrations")]
async fn only_accepted_direct_or_group_chats_and_follows_make_a_contact(pool: PgPool) {
    let chat = state(pool.clone(), None);
    conversation(&pool, "direct", &[("1", true), ("2", false)]).await;
    conversation(&pool, "room", &[("1", true), ("2", true)]).await;
    assert_eq!(
        relation(&chat, "2", "1").await.expect("read"),
        Relation {
            contact: false,
            follows: false
        }
    );
    conversation(&pool, "group", &[("1", true), ("2", true)]).await;
    assert!(relation(&chat, "2", "1").await.expect("read").contact);

    sqlx::query(
        "INSERT INTO user_followings (user_id, target_user_urn) VALUES ('soundcloud:users:4', 'soundcloud:users:3')",
    )
    .execute(&pool)
    .await
    .expect("a follow");
    assert!(relation(&chat, "4", "3").await.expect("read").follows);
    assert!(!relation(&chat, "3", "4").await.expect("read").follows);
}

#[sqlx::test(migrations = "./migrations")]
async fn blocks_make_every_user_unavailable(pool: PgPool) {
    let chat = state(pool.clone(), None);
    open_dms(&pool, "2").await;
    assert_eq!(
        evaluate(&chat, "1", "2").await.expect("read"),
        Landing::Request
    );
    sqlx::query("INSERT INTO chat.blocks (user_id, blocked_user) VALUES ('2', '1')")
        .execute(&pool)
        .await
        .expect("a block");
    assert_eq!(
        evaluate(&chat, "1", "2").await.expect("read"),
        Landing::Unavailable
    );
    assert_eq!(
        evaluate(&chat, "2", "1").await.expect("read"),
        Landing::Unavailable
    );
    assert_eq!(
        evaluate(&chat, "1", "1").await.expect("read"),
        Landing::Direct
    );
}

async fn session(pool: &PgPool, age: Duration) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO sessions (id, created_at) VALUES ($1, (now() - $2::int * interval '1 second') AT TIME ZONE current_setting('TimeZone'))")
        .bind(id)
        .bind(age.num_seconds() as i32)
        .execute(pool)
        .await
        .expect("a session");
    id
}

fn from(user: &str, session: Uuid) -> ChatCaller {
    let mut caller = caller();
    caller.user_id = user.to_owned();
    caller.session_id = session;
    caller
}

#[sqlx::test(migrations = "./migrations")]
async fn a_fresh_session_or_a_restriction_cannot_open_a_request(pool: PgPool) {
    let chat = state(pool.clone(), None);
    open_dms(&pool, "2").await;
    let young = session(&pool, Duration::minutes(10)).await;
    let old = session(&pool, Duration::hours(3)).await;
    assert!(session_is_young(&chat, young).await.expect("read"));
    assert!(!session_is_young(&chat, old).await.expect("read"));
    assert!(!session_is_young(&chat, Uuid::now_v7()).await.expect("read"));

    assert_eq!(
        landing(&chat, &from("1", young), "2").await,
        Err(ChatError::TooYoung)
    );
    users::restrict(&chat, "1", Utc::now() + Duration::hours(1), &[Scope::DmNew])
        .await
        .expect("restricted");
    assert!(matches!(
        landing(&chat, &from("1", old), "2").await,
        Err(ChatError::Muted { .. })
    ));
    conversation(&pool, "direct", &[("1", true), ("2", true)]).await;
    assert_eq!(
        landing(&chat, &from("1", young), "2").await,
        Ok(Landing::Direct)
    );
    users::restrict(&chat, "1", Utc::now() + Duration::hours(1), &[Scope::All])
        .await
        .expect("banned");
    assert!(matches!(
        landing(&chat, &from("1", old), "2").await,
        Err(ChatError::Muted { .. })
    ));
    assert_eq!(
        landing(&chat, &from("3", old), "4").await,
        Err(ChatError::Unavailable)
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_pending_dm_takes_three_messages_until_it_is_accepted(pool: PgPool) {
    let keys = TestKeys::default();
    let chat = state(pool.clone(), None);
    let dm = conversation(&pool, "direct", &[("1", true), ("2", false)]).await;
    let ck = conversation_key(&pool, &keys, dm).await;
    for index in 0..MESSAGES_BEFORE_ACCEPT {
        check_send(&chat, dm, "1").await.expect("allowed");
        post(&pool, dm, &ck, "1", &format!("hi {index}")).await;
    }
    assert_eq!(
        check_send(&chat, dm, "1").await,
        Err(ChatError::RequestLimit)
    );

    let group = conversation(&pool, "group", &[("1", true), ("2", false)]).await;
    let group_ck = conversation_key(&pool, &keys, group).await;
    for _ in 0..5 {
        post(&pool, group, &group_ck, "1", "news").await;
    }
    check_send(&chat, group, "1")
        .await
        .expect("groups have no request cap");

    users::restrict(&chat, "1", Utc::now() + Duration::hours(1), &[Scope::DmNew])
        .await
        .expect("restricted");
    let fresh = conversation(&pool, "direct", &[("1", true), ("5", false)]).await;
    assert!(matches!(
        check_send(&chat, fresh, "1").await,
        Err(ChatError::Muted { .. })
    ));
    sqlx::query(
        "UPDATE chat.members SET accepted = true, state = 'active' WHERE conversation_id = $1",
    )
    .bind(dm)
    .execute(&pool)
    .await
    .expect("accepted");
    check_send(&chat, dm, "1")
        .await
        .expect("an accepted chat is open");
    users::restrict(&chat, "1", Utc::now() + Duration::hours(1), &[Scope::All])
        .await
        .expect("banned");
    assert!(matches!(
        check_send(&chat, dm, "1").await,
        Err(ChatError::Muted { .. })
    ));
}

#[sqlx::test(migrations = "./migrations")]
async fn requests_list_pending_chats_without_blocked_senders(pool: PgPool) {
    let chat = state(pool.clone(), None);
    let wanted = conversation(&pool, "direct", &[("7", true), ("101", false)]).await;
    conversation(&pool, "group", &[("8", true), ("101", false)]).await;
    conversation(&pool, "direct", &[("9", true), ("101", true)]).await;
    let blocked = conversation(&pool, "direct", &[("10", true), ("101", false)]).await;
    sqlx::query("INSERT INTO chat.blocks (user_id, blocked_user) VALUES ('101', '10')")
        .execute(&pool)
        .await
        .expect("a block");

    let shown = list(caller(), State(chat.clone()))
        .await
        .expect("requests")
        .0;
    assert_eq!(shown.count, 2);
    let ids: Vec<Uuid> = shown.requests.iter().map(|r| r.conversation_id).collect();
    assert!(ids.contains(&wanted));
    assert!(!ids.contains(&blocked));
    let from: Vec<&str> = shown.requests.iter().map(|r| r.from.as_str()).collect();
    assert!(from.contains(&"7") && from.contains(&"8"));
    assert_eq!(count(&chat, "101").await.expect("read"), 2);

    let tight = ChatCaller {
        max_bytes: 300,
        ..caller()
    };
    let squeezed = list(tight, State(chat.clone())).await.expect("requests").0;
    assert_eq!((squeezed.count, squeezed.requests.len()), (2, 1));
}
