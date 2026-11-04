use std::sync::Arc;

use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use super::backdrop_support::{Bench, profile, someone};
use crate::modules::chat::ChatState;
use crate::modules::chat::b64;
use crate::modules::chat::devices::test_support::{Enrolled, age, call, caller, enroll};
use crate::modules::rooms::model::Backdrop;
use crate::modules::rooms::service::Cause;

async fn keys(
    chat: &Arc<ChatState>,
    user: &str,
    device: &Enrolled,
    conversation: Uuid,
) -> Vec<i64> {
    let (status, answer) = call(
        chat,
        caller(user, device.session, Some(&device.row)),
        "GET",
        &format!("/chat/conversations/{conversation}/keys"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    answer
        .as_array()
        .map(|keys| {
            keys.iter()
                .filter_map(|key| key["epoch"].as_i64())
                .collect()
        })
        .unwrap_or_default()
}

async fn backdrop(
    chat: &Arc<ChatState>,
    code: &str,
    backdrop: Option<Backdrop>,
) -> anyhow::Result<()> {
    chat.rooms
        .apply(code, Cause::Quiet, |room| {
            room.backdrop = backdrop.clone();
            Ok(())
        })
        .await?;
    Ok(())
}

async fn meta(
    chat: &Arc<ChatState>,
    user: &str,
    device: &Enrolled,
    conversation: Uuid,
    epoch: i64,
    body: &[u8],
) -> (StatusCode, Value) {
    call(
        chat,
        caller(user, device.session, Some(&device.row)),
        "POST",
        &format!("/chat/conversations/{conversation}/messages"),
        Some(json!({
            "client_id": Uuid::now_v7(),
            "class": "meta",
            "epoch": epoch,
            "target_seq": null,
            "body": b64::encode(body),
            "media": []
        })),
    )
    .await
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn only_the_host_sends_room_meta_and_members_read_it_back(pg: PgPool) -> anyhow::Result<()> {
    let bench = Bench::new(pg.clone())?;
    let chat = bench.chat.clone();
    let (host, moderator, guest, stranger) = (someone(), someone(), someone(), someone());
    bench.star(&host).await?;
    let code = bench.room(&host, true, &[&moderator, &guest]).await?;
    chat.rooms
        .set_moderator(&code, &host, &moderator, true)
        .await?;
    let conversation = bench.open_chat(&code, &host).await?;
    let host_device = enroll(&chat, &pg, &host).await;
    let moderator_device = enroll(&chat, &pg, &moderator).await;
    let guest_device = enroll(&chat, &pg, &guest).await;
    let epoch = *keys(&chat, &host, &host_device, conversation)
        .await
        .last()
        .expect("a key");

    let sealed = vec![7_u8; 1_104];
    let (status, sent) = meta(&chat, &host, &host_device, conversation, epoch, &sealed).await;
    assert_eq!(status, StatusCode::OK, "{sent}");
    for (user, device) in [(&moderator, &moderator_device), (&guest, &guest_device)] {
        let (status, answer) = meta(&chat, user, device, conversation, epoch, b"nope").await;
        assert_eq!(
            (status, answer),
            (StatusCode::FORBIDDEN, json!({"error": "chat_not_member"}))
        );
    }
    chat.rooms
        .reconfigure(
            &code,
            &host,
            &serde_json::from_value(json!({"chat_enabled": false}))?,
        )
        .await?;
    let (status, sent) = meta(&chat, &host, &host_device, conversation, epoch, &sealed).await;
    assert_eq!(status, StatusCode::OK, "{sent}");

    for user in [&host, &guest] {
        let (status, view) = bench
            .ask(
                user,
                "GET",
                &format!("/chat/conversations/{conversation}"),
                None,
            )
            .await?;
        assert_eq!(status, StatusCode::OK, "{view}");
        assert_eq!(view["meta_ct"], json!(b64::encode(&sealed)));
        assert_eq!(view["meta_epoch"], json!(epoch));
        assert_eq!(view["kind"], json!("room"));
        assert_eq!(view["room_ref"], json!(code));
        assert_eq!(view["me"]["state"], json!("active"));
    }
    let (_, view) = bench
        .ask(
            &host,
            "GET",
            &format!("/chat/conversations/{conversation}"),
            None,
        )
        .await?;
    assert_eq!(view["me"]["role"], json!("owner"));
    assert_eq!(view["member_count"], json!(3));
    assert_eq!(
        bench
            .ask(
                &stranger,
                "GET",
                &format!("/chat/conversations/{conversation}"),
                None
            )
            .await?,
        (StatusCode::FORBIDDEN, json!({"error": "chat_not_member"}))
    );
    chat.rooms.end(&code).await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_late_joiner_gets_the_meta_key_and_nothing_else_below_its_floor(
    pg: PgPool,
) -> anyhow::Result<()> {
    let bench = Bench::new(pg.clone())?;
    let chat = bench.chat.clone();
    let (host, first, second, late, stays) =
        (someone(), someone(), someone(), someone(), someone());
    bench.star(&host).await?;
    let code = bench.room(&host, false, &[&first, &second, &stays]).await?;
    let conversation = bench.open_chat(&code, &host).await?;
    let image = Backdrop::Image {
        media_id: Uuid::now_v7(),
    };
    backdrop(&chat, &code, Some(image)).await?;
    let host_device = enroll(&chat, &pg, &host).await;
    let meta_epoch = *keys(&chat, &host, &host_device, conversation)
        .await
        .last()
        .expect("a key");
    let (status, sent) = meta(
        &chat,
        &host,
        &host_device,
        conversation,
        meta_epoch,
        b"backdrop",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{sent}");

    chat.rooms.kick(&code, &host, &first, false).await?;
    let middle = *keys(&chat, &host, &host_device, conversation)
        .await
        .last()
        .expect("a key");
    chat.rooms.kick(&code, &host, &second, false).await?;
    let newest = *keys(&chat, &host, &host_device, conversation)
        .await
        .last()
        .expect("a key");
    assert!(meta_epoch < middle && middle < newest);

    let new_phone = enroll(&chat, &pg, &stays).await;
    let seen = keys(&chat, &stays, &new_phone, conversation).await;
    assert!(
        seen.contains(&meta_epoch) && seen.contains(&newest),
        "a new device of an old member draws the backdrop too: {seen:?}"
    );

    chat.rooms
        .join(&code, &late, &profile("Late"), &chat.pg)
        .await?;
    let fresh = enroll(&chat, &pg, &late).await;
    assert!(
        keys(&chat, &late, &fresh, conversation)
            .await
            .contains(&meta_epoch),
        "a brand new device still draws the backdrop"
    );
    let late_device = Enrolled {
        row: age(&chat, &pg, &fresh.row, 100).await,
        ..fresh
    };
    assert_eq!(
        keys(&chat, &late, &late_device, conversation).await,
        [meta_epoch, newest]
    );
    let (status, answer) = call(
        &chat,
        caller(&late, late_device.session, Some(&late_device.row)),
        "GET",
        &format!(
            "/chat/conversations/{conversation}/keys?from_epoch={}",
            newest
        ),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer.as_array().map(Vec::len), Some(1));
    assert_eq!(
        keys(&chat, &host, &host_device, conversation).await,
        [meta_epoch, middle, newest]
    );
    let aurora = Backdrop::Preset {
        id: "aurora".to_owned(),
    };
    backdrop(&chat, &code, Some(aurora)).await?;
    assert_eq!(
        keys(&chat, &late, &late_device, conversation).await,
        [newest],
        "without an image the meta key stays with the members who had it"
    );
    chat.rooms.end(&code).await?;
    Ok(())
}
