use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;

use super::backdrop_support::{Bench, profile, put, someone};
use super::media_gate::{backdrop_gate, image_gate};
use crate::modules::chat::ChatError;
use crate::modules::rooms::model::{FREE_CAPACITY, Perks};
use crate::modules::rooms::service::Cause;

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_host_who_buys_star_mid_room_gets_the_perks_on_the_first_try(
    pg: PgPool,
) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let (host, guest) = (someone(), someone());
    let code = bench.room(&host, false, &[&guest]).await?;
    let cover = json!({"cover": {"url": "https://i1.sndcdn.com/artworks-2.jpg", "accent": 1}});
    assert_eq!(
        put(&bench, &host, &code, cover.clone()).await?,
        (
            StatusCode::FORBIDDEN,
            json!({"error": "chat_room_star_required"})
        )
    );
    bench.star(&host).await?;
    let (status, view) = put(&bench, &host, &code, cover).await?;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(
        view["perks"],
        json!({"starHost": true, "capacity": 25, "modsMax": 8})
    );

    let (other, member) = (someone(), someone());
    let free = bench.room(&other, false, &[&member]).await?;
    let conversation = bench.open_chat(&free, &other).await?;
    assert_eq!(
        image_gate(&bench.chat, conversation, &member).await,
        Err(ChatError::RoomStarRequired)
    );
    assert_eq!(
        backdrop_gate(&bench.chat, conversation, &other).await,
        Err(ChatError::RoomStarRequired)
    );
    bench.star(&other).await?;
    assert_eq!(
        image_gate(&bench.chat, conversation, &member).await,
        Err(ChatError::RoomStarRequired),
        "only the host's own try checks STAR live"
    );
    assert_eq!(
        backdrop_gate(&bench.chat, conversation, &other).await,
        Ok(())
    );
    assert!(bench.chat.rooms.room(&free).await?.perks.star_host);
    bench.chat.rooms.end(&code).await?;
    bench.chat.rooms.end(&free).await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_lapsed_room_over_ten_admits_nobody_and_drops_nobody(pg: PgPool) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let host = someone();
    bench.star(&host).await?;
    let guests: Vec<String> = (0..11).map(|_| someone()).collect();
    let names: Vec<&str> = guests.iter().map(String::as_str).collect();
    let code = bench.room(&host, false, &names).await?;
    let long_ago = chrono::Utc::now().timestamp_millis() - 10 * 60_000;
    bench
        .chat
        .rooms
        .apply(&code, Cause::Quiet, |room| {
            room.perks = Perks::free();
            for member in &mut room.members {
                member.joined_at = long_ago;
            }
            Ok(())
        })
        .await?;
    let mut conn = bench.chat.redis.get().await?;
    for gone in &guests[..4] {
        let _: i64 = redis::cmd("ZREM")
            .arg(format!("rooms:{code}:seen"))
            .arg(gone)
            .query_async(&mut conn)
            .await?;
    }
    drop(conn);
    bench
        .chat
        .rooms
        .set_moderator(&code, &host, &guests[0], true)
        .await?;

    let newcomer = someone();
    let refused = bench
        .chat
        .rooms
        .join(&code, &newcomer, &profile("New"), &bench.pg)
        .await;
    assert!(
        matches!(&refused, Err(error) if error.public_code() == "room_full"),
        "{refused:?}"
    );
    let room = bench.chat.rooms.room(&code).await?;
    assert_eq!(room.members.len(), 12);
    assert!(room.members.len() > FREE_CAPACITY);
    assert_eq!(room.moderators, [guests[0].clone()]);
    assert!(!room.is_member(&newcomer));
    bench.chat.rooms.end(&code).await?;
    Ok(())
}
