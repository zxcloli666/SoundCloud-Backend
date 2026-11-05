use sc_chat_wire::msg::Class;
use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::chat::conversations::fixtures::{
    DEVICE, access_of, group, key_epoch, outgoing, seqs,
};
use crate::modules::chat::media::refs;
use crate::modules::chat::media::rows::{Seed, refs as refs_of, seed};
use crate::modules::chat::media::support::{bench, until_blocked, user};
use crate::modules::chat::members::store as members;
use crate::modules::chat::messages::store;
use crate::modules::chat::model::MemberState;
use crate::modules::chat::{ChatError, ChatState};

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

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_member_removed_while_forwarding_cannot_forward(pool: PgPool) {
    let chat = bench(&pool);
    let (owner, forwarder) = (user(), user());
    let source = group(&pool, &owner, &[(&forwarder, MemberState::Active)]).await;
    let target = group(&pool, &forwarder, &[]).await;
    for kicked in [false, true] {
        let image = posted_image(&chat, source, &owner).await;
        let access = access_of(&pool, target, &forwarder).await;
        let mut out = outgoing(Class::Post, key_epoch(&pool, target).await, None);
        out.media = vec![image];
        refs::admit_send(&chat, &access, &out)
            .await
            .expect("the early check passes");
        let posts = seqs(&pool, target).await;
        let mut admin = pool.begin().await.expect("a tx");
        members::lock(&mut admin, source)
            .await
            .expect("a lock")
            .expect("a live source");
        if kicked {
            members::set_state(
                &mut admin,
                source,
                &forwarder,
                MemberState::Kicked,
                &[MemberState::Active],
            )
            .await
            .expect("a kick")
            .expect("a kicked member");
        }
        let sending = {
            let pool = pool.clone();
            tokio::spawn(async move { store::store(&pool, &access, DEVICE, &out, None).await })
        };
        until_blocked(&pool).await;
        assert!(!sending.is_finished());
        admin.commit().await.expect("a commit");
        let sent = sending.await.expect("a task").map(|row| row.seq);
        let in_target = refs_of(&pool, image)
            .await
            .into_iter()
            .filter(|(conversation, _)| *conversation == target)
            .count();
        if kicked {
            assert_eq!(sent, Err(ChatError::MediaMissing { ids: vec![image] }));
            assert_eq!(seqs(&pool, target).await, posts);
            assert_eq!(in_target, 0);
        } else {
            assert!(sent.is_ok(), "{sent:?}");
            assert_eq!(in_target, 1);
        }
    }
}
