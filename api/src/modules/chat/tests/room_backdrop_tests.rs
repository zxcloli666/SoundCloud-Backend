use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::backdrop_support::{BLOCK, Bench, image, missing, preset, put, someone};
use super::media_gate::backdrop_gate;
use crate::modules::chat::ChatError;
use crate::modules::chat::media::backdrop;
use crate::modules::rooms::model::IMAGE_COOLDOWN_MS;

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn the_settings_set_and_replace_the_backdrop_and_its_ref(pg: PgPool) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let (host, guest) = (someone(), someone());
    bench.star(&host).await?;
    bench.star(&guest).await?;
    let code = bench.room(&host, false, &[&guest]).await?;
    let conversation = bench.open_chat(&code, &host).await?;
    let elsewhere = Uuid::now_v7();

    let (status, view) = put(&bench, &host, &code, preset("aurora")).await?;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["backdrop"], json!({"kind": "preset", "id": "aurora"}));
    assert_eq!(view["backdropCooldownUntil"], json!(null));
    assert_eq!(
        put(&bench, &host, &code, preset("lava")).await?,
        (StatusCode::BAD_REQUEST, json!({"error": "chat_invalid"}))
    );
    assert_eq!(
        put(&bench, &guest, &code, preset("aurora")).await?.0,
        StatusCode::FORBIDDEN
    );

    let foreign = bench
        .upload(&guest, conversation, "backdrop", "uploaded")
        .await?;
    let other_room = bench
        .upload(&host, elsewhere, "backdrop", "uploaded")
        .await?;
    let pending = bench
        .upload(&host, conversation, "backdrop", "reserved")
        .await?;
    let picture = bench
        .upload(&host, conversation, "image", "uploaded")
        .await?;
    for wrong in [foreign, other_room, pending, picture, Uuid::new_v4()] {
        assert_eq!(
            put(&bench, &host, &code, image(wrong)).await?,
            missing(wrong)
        );
        bench.forget_limits(&code).await?;
    }
    assert!(bench.refs_of(conversation).await?.is_empty());

    let first = bench
        .upload(&host, conversation, "backdrop", "uploaded")
        .await?;
    bench.mark_meta(conversation).await?;
    let (status, view) = put(&bench, &host, &code, image(first)).await?;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["backdrop"], json!({"kind": "image", "mediaId": first}));
    assert_eq!(bench.refs_of(conversation).await?, [(first, 0)]);
    assert_eq!(bench.status_of(first).await?, "attached");
    assert_eq!(
        bench.meta_of(conversation).await?,
        None,
        "a new image waits for its meta"
    );
    bench.mark_meta(conversation).await?;
    let (status, _) = put(&bench, &host, &code, image(first)).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bench.refs_of(conversation).await?, [(first, 0)]);
    assert!(bench.meta_of(conversation).await?.is_some());

    let usage = bench.usage_of(&host).await?;
    let second = bench
        .upload(&host, conversation, "backdrop", "uploaded")
        .await?;
    let (status, _) = put(&bench, &host, &code, image(second)).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bench.refs_of(conversation).await?, [(second, 0)]);
    assert_eq!(bench.status_of(first).await?, "deleting");
    assert_eq!(bench.status_of(second).await?, "attached");
    assert_eq!(bench.usage_of(&host).await?, (usage.0, usage.1));
    assert_eq!(bench.meta_of(conversation).await?, None);

    bench.forget_limits(&code).await?;
    bench.mark_meta(conversation).await?;
    let (status, view) = put(&bench, &host, &code, preset("satin")).await?;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert!(bench.refs_of(conversation).await?.is_empty());
    assert_eq!(bench.status_of(second).await?, "deleting");
    assert_eq!(bench.usage_of(&host).await?, (usage.0 - BLOCK, usage.1 - 1));
    let (status, view) = put(&bench, &host, &code, json!({"backdrop": null})).await?;
    assert_eq!((status, &view["backdrop"]), (StatusCode::OK, &json!(null)));
    assert!(
        bench.meta_of(conversation).await?.is_some(),
        "only a new image forgets the meta"
    );
    bench.chat.rooms.end(&code).await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn the_seventh_backdrop_change_in_ten_minutes_is_refused(pg: PgPool) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let host = someone();
    bench.star(&host).await?;
    let code = bench.room(&host, true, &[]).await?;
    for id in ["aurora", "vinyl", "dunes", "satin", "paper", "candy-sky"] {
        let (status, view) = put(&bench, &host, &code, preset(id)).await?;
        assert_eq!(status, StatusCode::OK, "{view}");
    }
    assert_eq!(
        put(&bench, &host, &code, preset("watercolor")).await?,
        (
            StatusCode::TOO_MANY_REQUESTS,
            json!({"error": "chat_request_limit"})
        )
    );
    let (status, view) = put(&bench, &host, &code, json!({"title": "Still mine"})).await?;
    assert_eq!(
        (status, &view["title"]),
        (StatusCode::OK, &json!("Still mine"))
    );
    assert_eq!(
        view["backdrop"],
        json!({"kind": "preset", "id": "candy-sky"})
    );
    bench.chat.rooms.end(&code).await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_free_host_gets_every_room_setting_but_the_star_ones(pg: PgPool) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let (free, star) = (someone(), someone());
    bench.star(&star).await?;
    let code = bench.room(&free, true, &[]).await?;
    let refused = (
        StatusCode::FORBIDDEN,
        json!({"error": "chat_room_star_required"}),
    );
    let cover = json!({"cover": {"url": "https://i1.sndcdn.com/artworks-1.jpg", "accent": 3}});
    assert_eq!(put(&bench, &free, &code, cover.clone()).await?, refused);
    assert_eq!(put(&bench, &free, &code, preset("aurora")).await?, refused);
    let (status, view) = put(
        &bench,
        &free,
        &code,
        json!({"cover": null, "roomImages": "off", "backdrop": null}),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["roomImages"], json!("off"));
    assert_eq!(
        put(&bench, &free, &code, json!({"room_images": "members"})).await?,
        (StatusCode::BAD_REQUEST, json!({"error": "chat_invalid"}))
    );

    let starred = bench.room(&star, false, &[]).await?;
    let (status, view) = put(&bench, &star, &starred, cover).await?;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(
        view["cover"],
        json!({"url": "https://i1.sndcdn.com/artworks-1.jpg", "accent": 3})
    );
    assert_eq!(
        view["perks"],
        json!({"starHost": true, "capacity": 25, "modsMax": 8})
    );
    for bad in [
        json!({"url": "https://i1.sndcdn.com/a.jpg", "accent": 8}),
        json!({"url": "https://example.com/a.jpg", "accent": 1}),
        json!({"url": "http://i1.sndcdn.com/a.jpg", "accent": 1}),
    ] {
        assert_eq!(
            put(&bench, &star, &starred, json!({"cover": bad})).await?,
            (StatusCode::BAD_REQUEST, json!({"error": "chat_invalid"}))
        );
    }
    let (status, view) = put(&bench, &star, &starred, json!({"roomImages": "members"})).await?;
    assert_eq!(
        (status, &view["roomImages"]),
        (StatusCode::OK, &json!("members"))
    );
    bench.chat.rooms.end(&code).await?;
    bench.chat.rooms.end(&starred).await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_cleared_image_cools_down_while_presets_still_work(pg: PgPool) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let host = someone();
    bench.star(&host).await?;
    let code = bench.room(&host, true, &[]).await?;
    let conversation = bench.open_chat(&code, &host).await?;
    let first = bench
        .upload(&host, conversation, "backdrop", "uploaded")
        .await?;
    assert_eq!(
        put(&bench, &host, &code, image(first)).await?.0,
        StatusCode::OK
    );
    assert_eq!(
        backdrop::clear(&bench.chat, conversation, Uuid::new_v4(), None).await,
        Ok(false)
    );
    assert_eq!(bench.refs_of(conversation).await?, [(first, 0)]);
    assert_eq!(
        backdrop::clear(&bench.chat, conversation, first, None).await,
        Ok(true)
    );
    assert_eq!(
        backdrop::clear(&bench.chat, conversation, first, None).await,
        Ok(false)
    );
    assert!(bench.refs_of(conversation).await?.is_empty());
    assert_eq!(bench.status_of(first).await?, "deleting");
    let shown = bench.chat.rooms.view(&code, &host).await?;
    assert_eq!(
        (shown.backdrop, shown.backdrop_cooldown_until),
        (None, None)
    );

    let second = bench
        .upload(&host, conversation, "backdrop", "uploaded")
        .await?;
    assert_eq!(
        put(&bench, &host, &code, image(second)).await?.0,
        StatusCode::OK
    );
    let until = chrono::Utc::now().timestamp_millis() + IMAGE_COOLDOWN_MS;
    assert_eq!(
        backdrop::clear(&bench.chat, conversation, second, Some(until)).await,
        Ok(true)
    );
    let third = bench
        .upload(&host, conversation, "backdrop", "uploaded")
        .await?;
    let (status, body) = put(&bench, &host, &code, image(third)).await?;
    assert_eq!(
        (status, body),
        (
            StatusCode::FORBIDDEN,
            json!({"error": "chat_backdrop_cooldown", "until": until})
        )
    );
    assert_eq!(
        backdrop_gate(&bench.chat, conversation, &host).await,
        Err(ChatError::BackdropCooldown {
            until: until as u64
        })
    );
    let (status, view) = put(&bench, &host, &code, preset("ocean-rays")).await?;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["backdropCooldownUntil"], json!(until));
    assert!(bench.refs_of(conversation).await?.is_empty());
    assert_eq!(bench.status_of(third).await?, "uploaded");
    bench.chat.rooms.end(&code).await?;
    Ok(())
}
