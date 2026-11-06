use sc_chat_wire::msg::Class;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::chat::conversations::fixtures::{DEVICE, dm, group, key_epoch, outgoing};
use crate::modules::chat::media::refs;
use crate::modules::chat::media::rows::{
    Seed, add_ref, refs as refs_of, room_conversation, seed, status, usage,
};
use crate::modules::chat::media::support::{
    AVATAR, IMAGE, THUMB, bench, call, error, live_room, star, user,
};
use crate::modules::chat::members::access::{self, Access};
use crate::modules::chat::messages::rules;
use crate::modules::chat::messages::store;
use crate::modules::chat::model::{MemberState, MessageRow};
use crate::modules::chat::{ChatError, ChatResult, ChatState};

const TOTAL: i64 = (IMAGE + THUMB) as i64;

async fn send(
    chat: &ChatState,
    conversation: Uuid,
    user: &str,
    class: Class,
    media: &[Uuid],
) -> ChatResult<MessageRow> {
    let access = access::load_in(chat, conversation, user).await?;
    send_as(chat, &access, class, media).await
}

async fn send_as(
    chat: &ChatState,
    access: &Access,
    class: Class,
    media: &[Uuid],
) -> ChatResult<MessageRow> {
    let epoch = key_epoch(&chat.pg, access.conversation).await;
    let mut out = outgoing(class, epoch, None);
    out.media = media.to_vec();
    rules::may_post(access, out.class)?;
    refs::admit_send(chat, access, &out).await?;
    store::store(&chat.pg, access, DEVICE, &out, None).await
}

fn missing(ids: &[Uuid]) -> ChatError {
    ChatError::MediaMissing { ids: ids.to_vec() }
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_post_attaches_own_uploads_and_a_reader_may_forward_them(pool: PgPool) {
    let chat = bench(&pool);
    let (owner, friend, stranger) = (user(), user(), user());
    let direct = dm(&pool, &owner, &friend, MemberState::Active).await;
    let (image, thumb) = seed(&pool, Seed::image(&owner, direct, "uploaded")).await;
    let posted = send(&chat, direct, &owner, Class::Post, &[image])
        .await
        .expect("a post");
    assert_eq!(refs_of(&pool, image).await, [(direct, posted.seq)]);
    assert_eq!(status(&pool, image).await.as_deref(), Some("attached"));
    assert_eq!(
        status(&pool, thumb.unwrap()).await.as_deref(),
        Some("attached")
    );
    let shared = group(&pool, &friend, &[(&stranger, MemberState::Active)]).await;
    let forwarded = send(&chat, shared, &friend, Class::Post, &[image])
        .await
        .expect("a forward");
    assert_eq!(refs_of(&pool, image).await.len(), 2);
    assert_eq!(usage(&pool, &owner).await, (TOTAL, 0, 1));
    assert_eq!(usage(&pool, &friend).await, (0, 0, 0));
    let elsewhere = dm(&pool, &stranger, &user(), MemberState::Active).await;
    let onward = send(&chat, elsewhere, &stranger, Class::Post, &[image]).await;
    assert!(onward.is_ok() && forwarded.seq > 0, "{onward:?}");
    let outsider = user();
    let theirs = dm(&pool, &outsider, &user(), MemberState::Active).await;
    let refused = send(&chat, theirs, &outsider, Class::Post, &[image]).await;
    assert_eq!(refused.err(), Some(missing(&[image])));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn bad_rows_are_missing_and_nothing_is_stored(pool: PgPool) {
    let chat = bench(&pool);
    let (owner, other) = (user(), user());
    let group = group(&pool, &owner, &[(&other, MemberState::Active)]).await;
    let (good, _) = seed(&pool, Seed::image(&owner, group, "uploaded")).await;
    let (reserved, _) = seed(&pool, Seed::image(&owner, group, "reserved")).await;
    let (deleting, _) = seed(&pool, Seed::image(&owner, group, "deleting")).await;
    let (avatar, _) = seed(
        &pool,
        Seed {
            kind: "avatar",
            ..Seed::image(&owner, group, "uploaded")
        },
    )
    .await;
    let (foreign, _) = seed(&pool, Seed::image(&other, Uuid::new_v4(), "uploaded")).await;
    let unknown = Uuid::new_v4();
    let asked = [good, reserved, deleting, avatar, foreign, unknown];
    let refused = send(&chat, group, &owner, Class::Post, &asked).await;
    assert_eq!(refused.err(), Some(missing(&asked[1..])));
    assert!(refs_of(&pool, good).await.is_empty());
    assert_eq!(status(&pool, good).await.as_deref(), Some("uploaded"));
    let seqs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chat.messages WHERE conversation_id = $1 AND class = 'post'",
    )
    .bind(group)
    .fetch_one(&pool)
    .await
    .expect("a count");
    assert_eq!(seqs, 0);
    let eleven: Vec<Uuid> = (0..11).map(|_| Uuid::new_v4()).collect();
    let epoch = key_epoch(&pool, group).await;
    let mut out = outgoing(Class::Post, epoch, None);
    out.media = eleven;
    let input = crate::modules::chat::messages::SendMessage {
        client_id: out.client_id,
        class: "post".to_owned(),
        epoch,
        target_seq: None,
        body: out.body.clone(),
        media: out.media.clone(),
        pings: Vec::new(),
    };
    assert_eq!(rules::outgoing(input).err(), Some(ChatError::Invalid));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn room_images_stay_in_their_room_both_ways(pool: PgPool) {
    let chat = bench(&pool);
    let owner = user();
    star(&pool, &owner).await;
    let room = live_room(&chat, &owner, &[]).await.conversation;
    let group = group(&pool, &owner, &[]).await;
    let (roomed, _) = seed(&pool, Seed::image(&owner, room, "uploaded")).await;
    let (grouped, _) = seed(&pool, Seed::image(&owner, group, "uploaded")).await;
    let refused = send(&chat, group, &owner, Class::Post, &[roomed]).await;
    assert_eq!(refused.err(), Some(missing(&[roomed])));
    let in_room = access::load_in(&chat, room, &owner)
        .await
        .expect("a room access");
    let epoch = key_epoch(&pool, room).await;
    let mut out = outgoing(Class::Post, epoch, None);
    out.media = vec![grouped];
    assert_eq!(
        refs::admit_send(&chat, &in_room, &out).await.err(),
        Some(missing(&[grouped]))
    );
    out.media = vec![roomed];
    assert_eq!(refs::admit_send(&chat, &in_room, &out).await.err(), None);
    out.media = (0..5).map(|_| roomed).collect();
    assert_eq!(
        refs::admit_send(&chat, &in_room, &out).await.err(),
        Some(ChatError::Invalid)
    );
    let mut meta = outgoing(Class::Meta, epoch, None);
    meta.media = vec![roomed];
    assert_eq!(
        refs::admit_send(&chat, &in_room, &meta).await.err(),
        Some(missing(&[roomed]))
    );
    meta.media = Vec::new();
    assert_eq!(refs::admit_send(&chat, &in_room, &meta).await.err(), None);
    let ended = room_conversation(&pool, &owner).await;
    let (stale, _) = seed(&pool, Seed::image(&owner, ended, "uploaded")).await;
    let mut gone = in_room.clone();
    gone.conversation = ended;
    out.media = vec![stale];
    assert_eq!(
        refs::admit_send(&chat, &gone, &out).await.err(),
        Some(ChatError::NotMember)
    );
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_new_group_avatar_replaces_the_old_one_and_settles_it(pool: PgPool) {
    let chat = bench(&pool);
    let (owner, admin, member) = (user(), user(), user());
    let group = group(
        &pool,
        &owner,
        &[
            (&admin, MemberState::Active),
            (&member, MemberState::Active),
        ],
    )
    .await;
    sqlx::query(
        "UPDATE chat.members SET role = 'admin' WHERE conversation_id = $1 AND user_id = $2",
    )
    .bind(group)
    .bind(&admin)
    .execute(&pool)
    .await
    .expect("an admin");
    let avatar = |owner| Seed {
        kind: "avatar",
        ..Seed::image(owner, group, "uploaded")
    };
    let (first, _) = seed(&pool, avatar(&owner)).await;
    let (second, _) = seed(&pool, avatar(&admin)).await;
    let (elsewhere, _) = seed(
        &pool,
        Seed {
            origin: Uuid::new_v4(),
            ..avatar(&owner)
        },
    )
    .await;
    send(&chat, group, &owner, Class::Meta, &[first])
        .await
        .expect("a first avatar");
    assert_eq!(refs_of(&pool, first).await, [(group, 0)]);
    assert_eq!(status(&pool, first).await.as_deref(), Some("attached"));
    send(&chat, group, &admin, Class::Meta, &[first])
        .await
        .expect("the current avatar kept");
    assert_eq!(refs_of(&pool, first).await, [(group, 0)]);
    let refused = send(&chat, group, &owner, Class::Meta, &[elsewhere]).await;
    assert_eq!(refused.err(), Some(missing(&[elsewhere])));
    let refused = send(&chat, group, &owner, Class::Meta, &[second]).await;
    assert_eq!(refused.err(), Some(missing(&[second])));
    assert_eq!(
        send(&chat, group, &member, Class::Meta, &[]).await.err(),
        Some(ChatError::NotMember)
    );
    send(&chat, group, &admin, Class::Meta, &[second])
        .await
        .expect("a replaced avatar");
    assert_eq!(refs_of(&pool, second).await, [(group, 0)]);
    assert!(refs_of(&pool, first).await.is_empty());
    assert_eq!(status(&pool, first).await.as_deref(), Some("deleting"));
    assert_eq!(usage(&pool, &owner).await, (AVATAR as i64, 0, 1));
    assert_eq!(usage(&pool, &admin).await, (AVATAR as i64, 0, 1));
    send(&chat, group, &owner, Class::Meta, &[])
        .await
        .expect("a dropped avatar");
    assert!(refs_of(&pool, second).await.is_empty());
    assert_eq!(status(&pool, second).await.as_deref(), Some("deleting"));
    assert_eq!(usage(&pool, &admin).await, (0, 0, 0));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_retried_send_answers_the_stored_message_after_its_media_went(pool: PgPool) {
    let chat = bench(&pool);
    let (owner, friend) = (user(), user());
    let direct = dm(&pool, &owner, &friend, MemberState::Active).await;
    let (image, _) = seed(&pool, Seed::image(&owner, direct, "uploaded")).await;
    let epoch = key_epoch(&pool, direct).await;
    let body = json!({
        "client_id": Uuid::now_v7(), "class": "post", "epoch": epoch, "target_seq": null,
        "body": crate::modules::chat::b64::encode(&[0x5a; 64]), "media": [image]
    });
    let uri = format!("/chat/conversations/{direct}/messages");
    let first = call(&chat, &owner, "POST", &uri, Some(body.clone())).await;
    assert_eq!(first.0, 200, "{first:?}");
    call(
        &chat,
        &owner,
        "POST",
        "/chat/media/delete",
        Some(json!({"ids": [image]})),
    )
    .await;
    let again = call(&chat, &owner, "POST", &uri, Some(body.clone())).await;
    assert_eq!(again, first);
    let mut fresh = body;
    fresh["client_id"] = json!(Uuid::now_v7());
    let refused = call(&chat, &owner, "POST", &uri, Some(fresh)).await;
    assert_eq!(error(&refused), (409, "chat_media_missing"));
    assert_eq!(refused.1["ids"], json!([image]));
    let edit = json!({
        "client_id": Uuid::now_v7(), "class": "edit", "epoch": epoch, "target_seq": first.1["seq"],
        "body": crate::modules::chat::b64::encode(&[0x5a; 64]), "media": [image]
    });
    let refused = call(&chat, &owner, "POST", &uri, Some(edit)).await;
    assert_eq!(error(&refused), (400, "chat_invalid"));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_forward_needs_a_ref_above_the_readers_floor(pool: PgPool) {
    let chat = bench(&pool);
    let (owner, late) = (user(), user());
    let source = group(&pool, &owner, &[(&late, MemberState::Active)]).await;
    let (image, _) = seed(&pool, Seed::image(&owner, source, "attached")).await;
    add_ref(&pool, image, source, 3).await;
    sqlx::query(
        "UPDATE chat.members SET hidden_before_seq = 5 WHERE conversation_id = $1 AND user_id = $2",
    )
    .bind(source)
    .bind(&late)
    .execute(&pool)
    .await
    .expect("a hide floor");
    let target = dm(&pool, &late, &user(), MemberState::Active).await;
    let refused = send(&chat, target, &late, Class::Post, &[image]).await;
    assert_eq!(refused.err(), Some(missing(&[image])));
    add_ref(&pool, image, source, 7).await;
    send(&chat, target, &late, Class::Post, &[image])
        .await
        .expect("a forward above the floor");
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn an_unsent_upload_whose_origin_is_gone_cannot_be_posted(pool: PgPool) {
    let chat = bench(&pool);
    let (owner, friend) = (user(), user());
    let direct = dm(&pool, &owner, &friend, MemberState::Active).await;
    let purged = Uuid::new_v4();
    let (orphan, _) = seed(&pool, Seed::image(&owner, purged, "uploaded")).await;
    let refused = send(&chat, direct, &owner, Class::Post, &[orphan]).await;
    assert_eq!(refused.err(), Some(missing(&[orphan])));
    assert_eq!(status(&pool, orphan).await.as_deref(), Some("uploaded"));
    let (kept, _) = seed(&pool, Seed::image(&owner, purged, "attached")).await;
    let posted = send(&chat, direct, &owner, Class::Post, &[kept])
        .await
        .expect("an attached image from a purged group still forwards");
    assert!(refs_of(&pool, kept).await.contains(&(direct, posted.seq)));
}
