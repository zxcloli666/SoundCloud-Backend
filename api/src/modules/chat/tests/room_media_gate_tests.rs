use axum::http::StatusCode;
use sqlx::PgPool;
use uuid::Uuid;

use super::backdrop_support::{Bench, image, put, someone};
use super::media_gate::{backdrop_gate, backdrop_meta, image_gate};
use crate::modules::chat::ChatError;
use crate::modules::chat::media::backdrop;
use crate::modules::rooms::model::{Backdrop, Perks, RoomImages};
use crate::modules::rooms::service::Cause;

async fn policy(bench: &Bench, code: &str, images: RoomImages) -> anyhow::Result<()> {
    bench
        .chat
        .rooms
        .apply(code, Cause::Quiet, |room| {
            room.room_images = Some(images);
            Ok(())
        })
        .await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn the_media_gates_follow_the_room_rules(pg: PgPool) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let (host, moderator, guest, stranger, free) =
        (someone(), someone(), someone(), someone(), someone());
    bench.star(&host).await?;
    bench.star(&guest).await?;
    let code = bench.room(&host, false, &[&moderator, &guest]).await?;
    bench
        .chat
        .rooms
        .set_moderator(&code, &host, &moderator, true)
        .await?;
    let conversation = bench.open_chat(&code, &host).await?;
    let chat = &bench.chat;

    assert_eq!(
        image_gate(chat, conversation, &stranger).await,
        Err(ChatError::NotMember)
    );
    assert_eq!(
        image_gate(chat, Uuid::now_v7(), &host).await,
        Err(ChatError::NotMember)
    );
    for user in [&host, &guest] {
        assert_eq!(image_gate(chat, conversation, user).await, Ok(()));
    }
    assert_eq!(
        image_gate(chat, conversation, &moderator).await,
        Err(ChatError::Unavailable),
        "members means every STAR member"
    );
    policy(&bench, &code, RoomImages::Mods).await?;
    assert_eq!(
        image_gate(chat, conversation, &guest).await,
        Err(ChatError::Unavailable)
    );
    assert_eq!(image_gate(chat, conversation, &moderator).await, Ok(()));
    assert_eq!(image_gate(chat, conversation, &host).await, Ok(()));
    policy(&bench, &code, RoomImages::Off).await?;
    assert_eq!(
        image_gate(chat, conversation, &host).await,
        Err(ChatError::Unavailable)
    );
    policy(&bench, &code, RoomImages::Members).await?;
    bench.chat.rooms.set_public(&code, &host, true).await?;
    assert_eq!(
        image_gate(chat, conversation, &guest).await,
        Err(ChatError::Unavailable)
    );
    assert_eq!(image_gate(chat, conversation, &moderator).await, Ok(()));

    assert_eq!(
        backdrop_gate(chat, conversation, &guest).await,
        Err(ChatError::NotMember)
    );
    assert_eq!(
        backdrop_gate(chat, conversation, &moderator).await,
        Err(ChatError::NotMember)
    );
    assert_eq!(backdrop_gate(chat, conversation, &host).await, Ok(()));

    let media = bench
        .upload(&host, conversation, "backdrop", "uploaded")
        .await?;
    assert_eq!(
        put(&bench, &host, &code, image(media)).await?.0,
        StatusCode::OK
    );
    let other = Uuid::new_v4();
    assert_eq!(backdrop_meta(chat, conversation, &host, &[]).await, Ok(()));
    assert_eq!(
        backdrop_meta(chat, conversation, &host, &[media]).await,
        Ok(())
    );
    assert_eq!(
        backdrop_meta(chat, conversation, &host, &[other]).await,
        Err(ChatError::MediaMissing { ids: vec![other] })
    );
    assert_eq!(
        backdrop_meta(chat, conversation, &host, &[media, other]).await,
        Err(ChatError::Invalid)
    );
    assert_eq!(
        backdrop_meta(chat, conversation, &moderator, &[]).await,
        Err(ChatError::NotMember)
    );

    let free_code = bench.room(&free, false, &[&guest]).await?;
    let free_chat = bench.open_chat(&free_code, &free).await?;
    assert_eq!(
        image_gate(chat, free_chat, &guest).await,
        Err(ChatError::RoomStarRequired)
    );
    assert_eq!(
        backdrop_gate(chat, free_chat, &free).await,
        Err(ChatError::RoomStarRequired)
    );
    bench.chat.rooms.end(&code).await?;
    bench.chat.rooms.end(&free_code).await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn the_gates_follow_a_lapse_a_public_room_and_a_missing_backdrop(
    pg: PgPool,
) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let (host, moderator, guest) = (someone(), someone(), someone());
    bench.star(&host).await?;
    bench.star(&guest).await?;
    let code = bench.room(&host, true, &[&moderator, &guest]).await?;
    bench
        .chat
        .rooms
        .set_moderator(&code, &host, &moderator, true)
        .await?;
    let conversation = bench.open_chat(&code, &host).await?;
    let chat = &bench.chat;
    let gone = Uuid::now_v7();

    assert_eq!(
        image_gate(chat, conversation, &guest).await,
        Err(ChatError::Unavailable),
        "no policy in a public room means mods"
    );
    assert_eq!(image_gate(chat, conversation, &moderator).await, Ok(()));
    let media = Uuid::now_v7();
    for wanted in [
        None,
        Some(Backdrop::Preset {
            id: "paper".to_owned(),
        }),
    ] {
        stored(&bench, &code, wanted).await?;
        assert_eq!(
            backdrop_meta(chat, conversation, &host, &[media]).await,
            Err(ChatError::MediaMissing { ids: vec![media] })
        );
        assert_eq!(backdrop_meta(chat, conversation, &host, &[]).await, Ok(()));
    }
    stored(&bench, &code, Some(Backdrop::Image { media_id: media })).await?;
    lapse(&bench, &code, &host).await?;
    assert_eq!(
        image_gate(chat, conversation, &moderator).await,
        Err(ChatError::RoomStarRequired)
    );
    assert_eq!(
        backdrop_gate(chat, conversation, &host).await,
        Err(ChatError::RoomStarRequired)
    );
    assert_eq!(
        backdrop_meta(chat, conversation, &host, &[media]).await,
        Ok(()),
        "a lapse keeps the stored image"
    );
    assert_eq!(
        backdrop_gate(chat, gone, &host).await,
        Err(ChatError::NotMember)
    );
    assert_eq!(
        backdrop_meta(chat, gone, &host, &[]).await,
        Err(ChatError::NotMember)
    );
    assert_eq!(backdrop::clear(chat, gone, media, None).await, Ok(false));
    bench.chat.rooms.end(&code).await?;
    Ok(())
}

async fn stored(bench: &Bench, code: &str, backdrop: Option<Backdrop>) -> anyhow::Result<()> {
    bench
        .chat
        .rooms
        .apply(code, Cause::Quiet, |room| {
            room.backdrop = backdrop.clone();
            Ok(())
        })
        .await?;
    Ok(())
}

async fn lapse(bench: &Bench, code: &str, host: &str) -> anyhow::Result<()> {
    bench.unstar(host).await?;
    bench
        .chat
        .rooms
        .apply(code, Cause::Quiet, |room| {
            room.perks = Perks::free();
            Ok(())
        })
        .await?;
    Ok(())
}
