use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;

use super::backdrop_support::{Bench, someone};
use super::report_support::{file, holds, report, stage};

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
    assert!(bench.chat.rooms.backdrop_cooldown(&host).await?.is_some());
    assert!(bench.refs_of(conversation).await?.is_empty());
    assert_eq!(bench.status_of(media).await?, "held");
    assert_eq!(holds(&bench, media).await?.len(), 3);
    bench.chat.rooms.end(&stage.code).await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn reports_filed_while_private_never_clear_after_the_room_opens(
    pg: PgPool,
) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let (host, first, second, third) = (someone(), someone(), someone(), someone());
    let stage = stage(&bench, &host, false, &[&first, &second, &third]).await?;
    let (conversation, media) = (stage.conversation, stage.media);
    for user in [&first, &second] {
        assert_eq!(
            file(&bench, user, report(conversation, media)).await?.0,
            StatusCode::OK
        );
    }
    bench
        .chat
        .rooms
        .set_public(&stage.code, &host, true)
        .await?;
    assert_eq!(
        file(&bench, &third, report(conversation, media)).await?.0,
        StatusCode::OK
    );
    for user in [&first, &second] {
        assert_eq!(
            file(&bench, user, report(conversation, media)).await?.1["duplicate"],
            json!(true)
        );
    }
    let room = bench.chat.rooms.room(&stage.code).await?;
    assert_eq!(room.image_backdrop(), Some(media));
    assert_eq!(bench.chat.rooms.backdrop_cooldown(&host).await?, None);
    assert_eq!(bench.refs_of(conversation).await?, [(media, 0)]);
    let public: Vec<bool> = sqlx::query_scalar(
        "SELECT room_public FROM chat.reports WHERE media_id = $1 ORDER BY created_at",
    )
    .bind(media)
    .fetch_all(&bench.pg)
    .await?;
    assert_eq!(public, [false, false, true]);
    bench.chat.rooms.end(&stage.code).await?;
    Ok(())
}
