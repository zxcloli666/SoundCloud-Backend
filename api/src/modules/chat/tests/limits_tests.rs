use std::collections::HashSet;
use std::sync::Arc;

use uuid::Uuid;

use super::{
    Bucket, ROOM_BYTES_PER_MINUTE, ROOM_NEWCOMER_BODY_MAX, RoomPost, admit, admit_request,
    admit_room_post, admit_send,
};
use crate::modules::chat::moderation::testing::state_with;
use crate::modules::chat::{ChatError, ChatState};

const BUCKETS: [Bucket; 11] = [
    Bucket::Send,
    Bucket::Create,
    Bucket::Request,
    Bucket::Invite,
    Bucket::Report,
    Bucket::Device,
    Bucket::ReadMessages,
    Bucket::ReadList,
    Bucket::ReadKeys,
    Bucket::ReadCard,
    Bucket::Preview,
];

#[test]
fn every_bucket_has_windows_and_distinct_keys() {
    let mut names = HashSet::new();
    for bucket in BUCKETS {
        assert!(names.insert(bucket.name()));
        assert!(!bucket.windows().is_empty(), "{bucket:?}");
        let labels: HashSet<&str> = bucket.windows().iter().map(|w| w.label).collect();
        assert_eq!(labels.len(), bucket.windows().len(), "{bucket:?}");
    }
    assert_eq!(Bucket::Send.windows()[1].limit, 30);
    assert_eq!(Bucket::Report.windows()[0].limit, 20);
    assert_eq!(Bucket::ReadList.windows()[0].limit, 10);
}

fn chat() -> Arc<ChatState> {
    let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned());
    let redis = crate::redis::connect_url(&url).expect("a redis pool");
    let pg = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://scd@127.0.0.1:1/none")
        .expect("a lazy pool");
    state_with(pg, redis, None)
}

fn user() -> String {
    format!("{}", rand::random::<u32>() as u64 + 1_000_000_000)
}

#[tokio::test]
#[ignore = "requires a local Redis"]
async fn a_user_is_limited_across_all_of_their_sessions() {
    let chat = chat();
    let user = user();
    for _session in 0..2 {
        for _ in 0..5 {
            admit(&chat, Bucket::ReadList, &user)
                .await
                .expect("admitted");
        }
    }
    assert_eq!(
        admit(&chat, Bucket::ReadList, &user).await,
        Err(ChatError::ReadLimited)
    );
    admit(&chat, Bucket::ReadList, &self::user())
        .await
        .expect("someone else is admitted");
    admit(&chat, Bucket::ReadCard, &user)
        .await
        .expect("another bucket has its own budget");
}

#[tokio::test]
#[ignore = "requires a local Redis"]
async fn sends_have_a_per_conversation_cap_and_a_burst() {
    let chat = chat();
    let user = user();
    let first = Uuid::now_v7();
    for _ in 0..5 {
        admit_send(&chat, &user, first).await.expect("admitted");
    }
    assert_eq!(
        admit_send(&chat, &user, first).await,
        Err(ChatError::SendLimited)
    );
    let second = Uuid::now_v7();
    for _ in 0..5 {
        admit_send(&chat, &user, second).await.expect("admitted");
    }
    assert_eq!(
        admit_send(&chat, &user, Uuid::now_v7()).await,
        Err(ChatError::SendLimited)
    );
}

#[tokio::test]
#[ignore = "requires a local Redis"]
async fn newcomers_get_half_the_daily_requests() {
    let chat = chat();
    let newcomer = user();
    for _ in 0..10 {
        admit_request(&chat, &newcomer, true)
            .await
            .expect("admitted");
    }
    assert_eq!(
        admit_request(&chat, &newcomer, true).await,
        Err(ChatError::RequestLimit)
    );
    let settled = user();
    for _ in 0..20 {
        admit_request(&chat, &settled, false)
            .await
            .expect("admitted");
    }
    assert_eq!(
        admit_request(&chat, &settled, false).await,
        Err(ChatError::RequestLimit)
    );
}

#[tokio::test]
#[ignore = "requires a local Redis"]
async fn public_rooms_judge_posts_by_ciphertext_length() {
    let chat = chat();
    let user = user();
    let conversation = Uuid::now_v7();
    let post = |member_for_secs, body_len, public| RoomPost {
        conversation,
        public,
        member_for_secs,
        body_len,
    };
    admit_room_post(&chat, &user, &post(30, 50_000, false))
        .await
        .expect("private rooms are not judged");
    admit_room_post(&chat, &user, &post(30, ROOM_NEWCOMER_BODY_MAX, true))
        .await
        .expect("a short post from a newcomer");
    assert_eq!(
        admit_room_post(&chat, &user, &post(30, ROOM_NEWCOMER_BODY_MAX + 256, true)).await,
        Err(ChatError::SendLimited)
    );
    let long = 10 * 1024;
    admit_room_post(&chat, &user, &post(3_600, long, true))
        .await
        .expect("a long post from a member");
    assert!((ROOM_NEWCOMER_BODY_MAX + 3 * long) as u64 > ROOM_BYTES_PER_MINUTE);
    assert_eq!(
        admit_room_post(&chat, &user, &post(3_600, long * 2, true)).await,
        Err(ChatError::SendLimited)
    );
}
