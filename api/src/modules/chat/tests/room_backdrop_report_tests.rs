use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::backdrop_support::{Bench, someone};
use super::media_gate::clear_backdrop_if;
use super::report_support::{file, holds, report, stage};
use crate::modules::chat::ChatError;
use crate::modules::chat::admin::audit;
use crate::modules::chat::admin::resolve::{Resolve, apply};
use crate::modules::chat::moderation::evidence;
use crate::modules::chat::moderation::task;
use crate::modules::chat::moderation::testing::audit_actions;
use crate::modules::chat::reports::load;
use crate::modules::rooms::model::IMAGE_COOLDOWN_MS;

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_backdrop_report_holds_the_image_and_seals_the_room_meta(
    pg: PgPool,
) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let (host, first, second, third, stranger) =
        (someone(), someone(), someone(), someone(), someone());
    let stage = stage(&bench, &host, false, &[&first, &second, &third]).await?;
    let (conversation, media) = (stage.conversation, stage.media);

    let (status, filed) = file(&bench, &first, report(conversation, media)).await?;
    assert_eq!(status, StatusCode::OK, "{filed}");
    assert_eq!(filed["duplicate"], json!(false));
    let id: Uuid = serde_json::from_value(filed["id"].clone())?;
    let (status, again) = file(&bench, &first, report(conversation, media)).await?;
    assert_eq!(
        (status, &again["duplicate"], &again["id"]),
        (StatusCode::OK, &json!(true), &filed["id"])
    );
    assert_eq!(holds(&bench, media).await?, [id]);

    let row = load(&bench.chat, id)
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?
        .expect("a report");
    assert_eq!(
        (row.target_user.as_str(), row.seq_from, row.seq_to),
        (host.as_str(), 0, 0)
    );
    assert_eq!(row.comment.as_deref(), Some("ugly"));
    let (kind, stored_media): (String, Option<Uuid>) =
        sqlx::query_as("SELECT kind, media_id FROM chat.reports WHERE id = $1")
            .bind(id)
            .fetch_one(&bench.pg)
            .await?;
    assert_eq!((kind.as_str(), stored_media), ("backdrop", Some(media)));
    let version = u16::try_from(row.evidence_kek_version.expect("a version"))?;
    let bundle = evidence::open(&bench.chat.keks, version, &row.evidence.expect("evidence"))
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    assert_eq!(bundle.messages.len(), 1);
    assert_eq!(bundle.messages[0].class, "meta");
    assert_eq!(bundle.messages[0].body, stage.meta);
    assert_eq!(i64::from(bundle.messages[0].epoch), stage.meta_epoch);
    let epochs: Vec<i64> = bundle.keys.iter().map(|key| i64::from(key.epoch)).collect();
    assert_eq!(epochs, [stage.meta_epoch]);
    assert_eq!(bundle.keys[0].ck.len(), 32);

    let invalid = (StatusCode::BAD_REQUEST, json!({"error": "chat_invalid"}));
    assert_eq!(
        file(&bench, &host, report(conversation, media)).await?,
        invalid
    );
    assert_eq!(
        file(&bench, &stranger, report(conversation, media)).await?,
        (StatusCode::FORBIDDEN, json!({"error": "chat_not_member"}))
    );
    let other = Uuid::new_v4();
    assert_eq!(
        file(&bench, &second, report(conversation, other)).await?,
        (
            StatusCode::CONFLICT,
            json!({"error": "chat_media_missing", "ids": [other]})
        )
    );
    let mut ranged = report(conversation, media);
    ranged["seq_from"] = json!(1);
    ranged["seq_to"] = json!(1);
    assert_eq!(file(&bench, &second, ranged).await?, invalid);
    assert_eq!(
        file(&bench, &second, json!({"conversation_id": conversation, "seq_from": 1, "seq_to": 1, "media_id": media, "reason": "spam"})).await?,
        invalid
    );

    for user in [&second, &third] {
        let (status, filed) = file(&bench, user, report(conversation, media)).await?;
        assert_eq!(
            (status, &filed["duplicate"]),
            (StatusCode::OK, &json!(false))
        );
    }
    let view = bench.chat.rooms.view(&stage.code, &host).await?;
    assert_eq!(
        view.backdrop.and_then(|shown| shown.image()),
        Some(media),
        "a private room never clears"
    );
    assert_eq!(holds(&bench, media).await?.len(), 3);

    task::run_once(&bench.chat, &bench.chat.keks)
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    let judged: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chat.reports WHERE conversation_id = $1 AND handled_by IS NULL AND state = 'open'",
    )
    .bind(conversation)
    .fetch_one(&bench.pg)
    .await?;
    assert_eq!(judged, 3, "backdrop reports wait for an admin");

    assert_eq!(
        clear_backdrop_if(&bench.chat, conversation, media).await,
        Ok(true)
    );
    assert_eq!(bench.status_of(media).await?, "held");
    assert!(bench.refs_of(conversation).await?.is_empty());
    assert_eq!(
        file(&bench, &first, report(conversation, media)).await?.0,
        StatusCode::OK
    );
    assert_eq!(
        clear_backdrop_if(&bench.chat, Uuid::now_v7(), media).await,
        Ok(false)
    );
    bench.chat.rooms.end(&stage.code).await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn three_reporters_in_a_public_room_clear_the_image_for_an_hour(
    pg: PgPool,
) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let (host, first, second, third) = (someone(), someone(), someone(), someone());
    let stage = stage(&bench, &host, true, &[&first, &second, &third]).await?;
    let (conversation, media) = (stage.conversation, stage.media);
    for user in [&first, &second, &first] {
        assert_eq!(
            file(&bench, user, report(conversation, media)).await?.0,
            StatusCode::OK
        );
    }
    let shown = bench.chat.rooms.view(&stage.code, &host).await?;
    assert_eq!(shown.backdrop.and_then(|shown| shown.image()), Some(media));
    let before = chrono::Utc::now().timestamp_millis();
    assert_eq!(
        file(&bench, &third, report(conversation, media)).await?.0,
        StatusCode::OK
    );
    let cleared = bench.chat.rooms.view(&stage.code, &host).await?;
    assert_eq!(cleared.backdrop, None);
    let until = cleared.backdrop_cooldown_until.expect("a cooldown");
    assert!(until >= before + IMAGE_COOLDOWN_MS && until <= before + IMAGE_COOLDOWN_MS + 60_000);
    assert!(bench.refs_of(conversation).await?.is_empty());
    assert_eq!(bench.status_of(media).await?, "held");
    assert_eq!(holds(&bench, media).await?.len(), 3);
    assert_eq!(
        file(&bench, &third, report(conversation, media)).await?.1["duplicate"],
        json!(true)
    );
    let fresh = bench
        .upload(&host, conversation, "backdrop", "uploaded")
        .await?;
    assert_eq!(
        bench
            .ask(
                &host,
                "PUT",
                &format!("/chat/rooms/{}/settings", stage.code),
                Some(json!({"backdrop": {"kind": "image", "mediaId": fresh}})),
            )
            .await?,
        (
            StatusCode::FORBIDDEN,
            json!({"error": "chat_backdrop_cooldown", "until": until})
        )
    );
    assert_eq!(
        crate::modules::chat::rooms::media_gate::backdrop_gate(&bench.chat, conversation, &host)
            .await,
        Err(ChatError::BackdropCooldown {
            until: until as u64
        })
    );
    bench.chat.rooms.end(&stage.code).await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn three_reports_at_once_clear_the_image_exactly_once(pg: PgPool) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let (host, first, second, third) = (someone(), someone(), someone(), someone());
    let stage = stage(&bench, &host, true, &[&first, &second, &third]).await?;
    let (conversation, media) = (stage.conversation, stage.media);
    let (a, b, c) = tokio::join!(
        file(&bench, &first, report(conversation, media)),
        file(&bench, &second, report(conversation, media)),
        file(&bench, &third, report(conversation, media)),
    );
    for answer in [a?, b?, c?] {
        assert_eq!(
            (answer.0, &answer.1["duplicate"]),
            (StatusCode::OK, &json!(false))
        );
    }
    let room = bench.chat.rooms.room(&stage.code).await?;
    assert_eq!(room.backdrop, None);
    assert!(room.backdrop_cooldown_until.is_some());
    assert!(bench.refs_of(conversation).await?.is_empty());
    assert_eq!(bench.status_of(media).await?, "held");
    assert_eq!(holds(&bench, media).await?.len(), 3);
    bench.chat.rooms.end(&stage.code).await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn an_upheld_backdrop_report_clears_the_live_room_and_frees_the_image(
    pg: PgPool,
) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let (host, guest) = (someone(), someone());
    let stage = stage(&bench, &host, false, &[&guest]).await?;
    let (conversation, media) = (stage.conversation, stage.media);
    let (status, filed) = file(&bench, &guest, report(conversation, media)).await?;
    assert_eq!(status, StatusCode::OK, "{filed}");
    let id: Uuid = serde_json::from_value(filed["id"].clone())?;

    let clear: Resolve =
        serde_json::from_value(json!({"decision": "uphold", "clear_backdrop": true}))?;
    let upheld = apply(&bench.chat, "owner", id, &clear)
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    assert_eq!(
        (upheld.state.as_str(), upheld.media_id),
        ("actioned", Some(media))
    );
    let view = bench.chat.rooms.view(&stage.code, &host).await?;
    assert_eq!(view.backdrop, None);
    assert!(holds(&bench, media).await?.is_empty());
    assert!(bench.refs_of(conversation).await?.is_empty());
    assert_eq!(bench.status_of(media).await?, "deleting");
    let audits = audit_actions(&bench.pg, &audit::report(id)).await;
    assert!(
        audits
            .iter()
            .any(|(_, action, _)| action == "clear_backdrop"),
        "{audits:?}"
    );
    assert_eq!(
        apply(&bench.chat, "owner", id, &clear)
            .await
            .map(|summary| summary.state),
        Ok("actioned".to_owned())
    );
    bench.chat.rooms.end(&stage.code).await?;
    Ok(())
}
