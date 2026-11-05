use sc_chat_wire::msg::Class;
use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::chat::ChatState;
use crate::modules::chat::conversations::fixtures::{
    DEVICE, access_of, dm, group, key_epoch, outgoing,
};
use crate::modules::chat::conversations::patch::{self, ConversationPatch, SettingsPatch};
use crate::modules::chat::conversations::remove;
use crate::modules::chat::media::refs;
use crate::modules::chat::media::rows::{Seed, seed, status, usage};
use crate::modules::chat::media::support::{bench, user};
use crate::modules::chat::members::changes;
use crate::modules::chat::messages::store;
use crate::modules::chat::model::MemberState;
use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::sweeps::media::trim;

async fn posted_image(chat: &ChatState, conversation: Uuid, owner: &str) -> Uuid {
    let (image, _) = seed(&chat.pg, Seed::image(owner, conversation, "uploaded")).await;
    let access = access_of(&chat.pg, conversation, owner).await;
    let mut out = outgoing(Class::Post, key_epoch(&chat.pg, conversation).await, None);
    out.media = vec![image];
    refs::admit_send(chat, &access, &out)
        .await
        .expect("admitted media");
    store::store(&chat.pg, &access, DEVICE, &out, None)
        .await
        .expect("a post");
    image
}

async fn queued(pool: &PgPool, conversation: Uuid) -> bool {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM chat.media_trim WHERE conversation_id = $1)")
        .bind(conversation)
        .fetch_one(pool)
        .await
        .expect("the trim queue")
}

async fn trimmed(chat: &ChatState) {
    trim::run(chat, &mut Budget::new(trim::BATCHES))
        .await
        .expect("a trim");
}

async fn hide(pool: &PgPool, conversation: Uuid, user: &str) {
    remove::hide(pool, &access_of(pool, conversation, user).await)
        .await
        .expect("hidden");
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn an_invitee_declining_frees_what_everyone_else_hid(pool: PgPool) {
    let chat = bench(&pool);
    let (owner, member, invitee) = (user(), user(), user());
    let group = group(
        &pool,
        &owner,
        &[
            (&member, MemberState::Active),
            (&invitee, MemberState::Invited),
        ],
    )
    .await;
    let image = posted_image(&chat, group, &owner).await;
    hide(&pool, group, &owner).await;
    hide(&pool, group, &member).await;
    trimmed(&chat).await;
    assert_eq!(status(&pool, image).await.as_deref(), Some("attached"));
    assert!(!queued(&pool, group).await);

    changes::decline(&pool, &access_of(&pool, group, &invitee).await)
        .await
        .expect("declined");

    assert!(queued(&pool, group).await);
    trimmed(&chat).await;
    assert_eq!(status(&pool, image).await.as_deref(), Some("deleting"));
    assert_eq!(usage(&pool, &owner).await, (0, 0, 0));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn hiding_history_frees_what_only_a_newcomer_could_see(pool: PgPool) {
    let chat = bench(&pool);
    let (owner, newcomer) = (user(), user());
    let group = group(&pool, &owner, &[]).await;
    let image = posted_image(&chat, group, &owner).await;
    changes::add(
        &pool,
        &access_of(&pool, group, &owner).await,
        vec![(newcomer.clone(), MemberState::Active)],
    )
    .await
    .expect("added");
    hide(&pool, group, &owner).await;
    trimmed(&chat).await;
    assert_eq!(status(&pool, image).await.as_deref(), Some("attached"));
    assert!(!queued(&pool, group).await);

    let hidden = ConversationPatch {
        settings: Some(SettingsPatch {
            history_visible: Some(false),
            ..SettingsPatch::default()
        }),
        ..ConversationPatch::default()
    };
    patch::apply(&pool, &access_of(&pool, group, &owner).await, hidden)
        .await
        .expect("patched");

    assert!(queued(&pool, group).await);
    trimmed(&chat).await;
    assert_eq!(status(&pool, image).await.as_deref(), Some("deleting"));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn showing_history_or_other_settings_queue_nothing(pool: PgPool) {
    let owner = user();
    let group = group(&pool, &owner, &[]).await;
    for settings in [
        SettingsPatch {
            history_visible: Some(true),
            ..SettingsPatch::default()
        },
        SettingsPatch {
            admins_only: Some(true),
            ..SettingsPatch::default()
        },
    ] {
        let change = ConversationPatch {
            settings: Some(settings),
            ..ConversationPatch::default()
        };
        patch::apply(&pool, &access_of(&pool, group, &owner).await, change)
            .await
            .expect("patched");
        assert!(!queued(&pool, group).await);
    }
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_second_side_hiding_a_dm_frees_its_images_at_once(pool: PgPool) {
    let chat = bench(&pool);
    let (sender, peer) = (user(), user());
    let dm = dm(&pool, &sender, &peer, MemberState::Active).await;
    let image = posted_image(&chat, dm, &sender).await;
    hide(&pool, dm, &sender).await;
    trimmed(&chat).await;
    assert_eq!(status(&pool, image).await.as_deref(), Some("attached"));

    hide(&pool, dm, &peer).await;

    assert!(queued(&pool, dm).await);
    trimmed(&chat).await;
    assert_eq!(status(&pool, image).await.as_deref(), Some("deleting"));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_hide_by_a_stranger_queues_nothing(pool: PgPool) {
    let (owner, stranger) = (user(), user());
    let group = group(&pool, &owner, &[]).await;
    let mut access = access_of(&pool, group, &owner).await;
    access.user_id = stranger;
    assert!(remove::hide(&pool, &access).await.is_err());
    assert!(!queued(&pool, group).await);
}
