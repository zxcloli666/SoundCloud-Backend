use std::sync::atomic::Ordering;

use serde_json::{Value, json};
use sqlx::PgPool;

use super::mobile_tests::{Canned, Profile, SYNC_MAX, TRACK, USER, World, created_playlist, world};
use super::unclear_create_tests::run;

const COMMENT: &str = "soundcloud:comments:5";
const PLAYLIST: &str = "soundcloud:playlists:900";
const CREATE: &str = "playlist_create";

fn mix() -> Option<Value> {
    Some(json!({"playlist": {"title": "Mix", "sharing": "private", "tracks": ["5"]}}))
}

fn comment_parent() -> Option<Value> {
    Some(json!({"parent_urn": TRACK}))
}

async fn owned(world: &World) -> anyhow::Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM user_owned_playlists WHERE playlist_urn = $1)",
    )
    .bind(PLAYLIST)
    .fetch_one(&world.pool)
    .await?)
}

async fn attempted(world: &World) -> anyhow::Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT remote_attempted_generation IS NOT DISTINCT FROM generation FROM sync_queue",
    )
    .fetch_one(&world.pool)
    .await?)
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_linked_create_goes_through_mobile_and_the_local_playlist_gets_the_new_urn(
    pool: PgPool,
) -> anyhow::Result<()> {
    let world = world(pool, Profile::Active).await?;
    world
        .mobile
        .reply_with([Canned::Json(201, json!({"urn": PLAYLIST}))]);

    world.send(CREATE, "new:1", mix()).await?;

    let seen = world.mobile.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, "POST");
    assert!(seen[0].1.contains("/playlists"));
    let sent: Option<Value> = world
        .mobile
        .sent(0)
        .and_then(|(body, _)| serde_json::from_str(&body).ok());
    assert_eq!(
        sent,
        Some(json!({
            "playlist": {"title": "Mix", "public": false},
            "track_urns": ["soundcloud:tracks:5"],
        }))
    );
    assert!(world.apiv1.calls().is_empty());
    assert_eq!(world.queued().await?, 0);
    assert!(owned(&world).await?);
    let title: Option<String> = sqlx::query_scalar("SELECT title FROM playlists WHERE urn = $1")
        .bind(PLAYLIST)
        .fetch_optional(&world.pool)
        .await?;
    assert_eq!(title.as_deref(), Some("Mix"));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn an_unlinked_create_keeps_the_apiv1_path_and_never_touches_mobile(
    pool: PgPool,
) -> anyhow::Result<()> {
    let world = world(pool, Profile::Missing).await?;

    world.send(CREATE, "new:1", mix()).await?;

    assert_eq!(world.mobile.count(), 0);
    assert_eq!(world.mobile.tokens.load(Ordering::SeqCst), 0);
    assert_eq!(world.apiv1.calls(), ["POST /playlists"]);
    assert_eq!(world.queued().await?, 0);
    assert!(owned(&world).await?);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn an_unlinked_delete_and_a_playlist_like_keep_the_apiv1_path(
    pool: PgPool,
) -> anyhow::Result<()> {
    let world = world(pool, Profile::Missing).await?;

    world.send("playlist_delete", PLAYLIST, None).await?;
    world.send("like_track", TRACK, None).await?;

    assert_eq!(world.mobile.count(), 0);
    assert_eq!(
        world.apiv1.calls(),
        [
            "DELETE /playlists/soundcloud:playlists:900",
            "POST /likes/tracks/soundcloud:tracks:42"
        ]
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_create_falls_back_to_apiv1_only_when_the_mobile_write_surely_did_not_happen(
    pool: PgPool,
) -> anyhow::Result<()> {
    let relink = world(pool.clone(), Profile::RelinkRequired).await?;
    relink.send(CREATE, "new:1", mix()).await?;
    assert_eq!(relink.mobile.count(), 0);
    assert_eq!(relink.apiv1.calls(), ["POST /playlists"]);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_bot_check_on_a_create_sends_it_to_apiv1(pool: PgPool) -> anyhow::Result<()> {
    let world = world(pool, Profile::Active).await?;
    world.mobile.reply_with([Canned::Captcha]);

    world.send(CREATE, "new:1", mix()).await?;

    assert_eq!(world.mobile.count(), 1);
    assert_eq!(world.apiv1.calls(), ["POST /playlists"]);
    assert_eq!(world.queued().await?, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_401_that_stays_after_the_refresh_sends_the_create_to_apiv1(
    pool: PgPool,
) -> anyhow::Result<()> {
    let world = world(pool, Profile::Active).await?;
    world
        .mobile
        .reply_with([Canned::Status(401), Canned::Status(401)]);

    world.send(CREATE, "new:1", mix()).await?;

    assert_eq!(world.mobile.count(), 2);
    assert_eq!(world.credential().await?.0, "relink_required");
    assert_eq!(world.apiv1.calls(), ["POST /playlists"]);
    assert_eq!(world.queued().await?, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn an_unclear_mobile_create_is_checked_on_soundcloud_and_never_posted_to_apiv1(
    pool: PgPool,
) -> anyhow::Result<()> {
    let world = world(pool, Profile::Active).await?;
    world.mobile.reply_with([
        Canned::Status(502),
        Canned::Json(201, json!({"urn": PLAYLIST})),
    ]);

    world.send(CREATE, "new:1", mix()).await?;

    assert_eq!(world.mobile.count(), 1);
    assert!(world.apiv1.calls().is_empty());
    assert_eq!(world.row().await?.0, 1);
    assert!(attempted(&world).await?);

    run(&world.pool, &world.handler).await?;

    assert_eq!(world.mobile.count(), 2);
    assert_eq!(world.apiv1.calls(), ["GET /me/playlists"]);
    assert_eq!(world.queued().await?, 0);
    assert!(owned(&world).await?);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn an_unclear_mobile_create_found_on_soundcloud_is_finished_without_a_second_post(
    pool: PgPool,
) -> anyhow::Result<()> {
    let world = world(pool, Profile::Active).await?;
    world.mobile.reply_with([Canned::Status(500)]);

    world.send(CREATE, "new:1", mix()).await?;
    world.apiv1.list(vec![created_playlist()]);
    run(&world.pool, &world.handler).await?;

    assert_eq!(world.mobile.count(), 1);
    assert_eq!(world.apiv1.calls(), ["GET /me/playlists"]);
    assert_eq!(world.queued().await?, 0);
    assert!(owned(&world).await?);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_created_reply_without_a_urn_is_found_in_the_listing_and_linked(
    pool: PgPool,
) -> anyhow::Result<()> {
    let world = world(pool, Profile::Active).await?;
    world.mobile.reply_with([Canned::Json(201, json!({}))]);

    world.send(CREATE, "new:1", mix()).await?;

    assert_eq!(world.mobile.count(), 1);
    assert!(world.apiv1.calls().is_empty());
    assert_eq!(world.row().await?.0, 1);
    assert!(!world.row().await?.1);
    assert!(attempted(&world).await?);

    world.apiv1.list(vec![created_playlist()]);
    run(&world.pool, &world.handler).await?;

    assert_eq!(world.mobile.count(), 1);
    assert_eq!(world.apiv1.calls(), ["GET /me/playlists"]);
    assert_eq!(world.queued().await?, 0);
    assert!(owned(&world).await?);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_client_error_on_a_mobile_create_is_parked_without_apiv1(
    pool: PgPool,
) -> anyhow::Result<()> {
    let world = world(pool, Profile::Active).await?;
    world.mobile.reply_with([Canned::Status(400)]);

    world.send(CREATE, "new:1", mix()).await?;

    assert!(world.row().await?.1);
    assert!(world.apiv1.calls().is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_create_with_fields_mobile_does_not_send_goes_to_apiv1_for_a_linked_user(
    pool: PgPool,
) -> anyhow::Result<()> {
    let world = world(pool, Profile::Active).await?;
    let described = Some(json!({"playlist": {"title": "Mix", "description": "d"}}));

    world.send(CREATE, "new:1", described).await?;

    assert_eq!(world.mobile.count(), 0);
    assert_eq!(world.apiv1.calls(), ["POST /playlists"]);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_linked_playlist_delete_is_a_mobile_delete_and_a_gone_playlist_closes_it(
    pool: PgPool,
) -> anyhow::Result<()> {
    let world = world(pool, Profile::Active).await?;
    world
        .mobile
        .reply_with([Canned::Status(200), Canned::Status(404)]);

    world.send("playlist_delete", PLAYLIST, None).await?;
    world
        .send("playlist_delete", "soundcloud:playlists:901", None)
        .await?;

    let seen = world.mobile.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].0, "DELETE");
    assert!(seen[0].1.contains("/playlists/soundcloud:playlists:900"));
    assert!(world.apiv1.calls().is_empty());
    assert_eq!(world.queued().await?, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_bot_check_on_a_playlist_delete_sends_it_to_apiv1(pool: PgPool) -> anyhow::Result<()> {
    let world = world(pool, Profile::Active).await?;
    world.mobile.reply_with([Canned::Captcha]);

    world.send("playlist_delete", PLAYLIST, None).await?;

    assert_eq!(
        world.apiv1.calls(),
        ["DELETE /playlists/soundcloud:playlists:900"]
    );
    assert_eq!(world.queued().await?, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_linked_like_is_a_mobile_post_and_an_unlike_is_a_mobile_delete_post(
    pool: PgPool,
) -> anyhow::Result<()> {
    let world = world(pool, Profile::Active).await?;
    world
        .mobile
        .reply_with([Canned::Status(201), Canned::Status(200)]);

    world.send("like_track", TRACK, None).await?;
    world.send("unlike_track", TRACK, None).await?;

    let seen = world.mobile.seen();
    assert_eq!(seen.len(), 2);
    assert!(seen[0].1.contains("/likes/tracks/create"));
    assert!(seen[1].1.contains("/likes/tracks/delete"));
    let sent: Vec<Option<Value>> = (0..2)
        .map(|index| {
            world
                .mobile
                .sent(index)
                .and_then(|(body, _)| serde_json::from_str(&body).ok())
        })
        .collect();
    for body in sent {
        assert_eq!(body, Some(json!({"likes": [{"target_urn": TRACK}]})));
    }
    assert!(world.apiv1.calls().is_empty());
    assert_eq!(world.queued().await?, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn an_unlinked_like_keeps_the_apiv1_path_and_never_touches_mobile(
    pool: PgPool,
) -> anyhow::Result<()> {
    let world = world(pool, Profile::Missing).await?;

    world.send("like_track", TRACK, None).await?;

    assert_eq!(world.mobile.count(), 0);
    assert_eq!(world.mobile.tokens.load(Ordering::SeqCst), 0);
    assert_eq!(
        world.apiv1.calls(),
        ["POST /likes/tracks/soundcloud:tracks:42"]
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_bot_check_on_a_like_goes_to_apiv1_at_once_and_the_next_likes_skip_mobile_for_the_hour(
    pool: PgPool,
) -> anyhow::Result<()> {
    let world = world(pool, Profile::Active).await?;
    world.mobile.reply_with([Canned::Captcha]);

    world.send("like_track", TRACK, None).await?;
    assert_eq!(world.mobile.count(), 1);
    assert_eq!(world.apiv1.calls().len(), 1);
    assert_eq!(world.credential().await?.0, "active");

    let app: uuid::Uuid = sqlx::query_scalar("SELECT oauth_app_id FROM soundcloud_connections")
        .fetch_one(&world.pool)
        .await?;
    world
        .handler
        .spacings
        .succeeded(&(app, "like_track".to_owned()), None);
    world
        .send("like_track", "soundcloud:tracks:43", None)
        .await?;
    assert_eq!(world.mobile.count(), 1);
    assert_eq!(world.apiv1.calls().len(), 2);
    assert!(
        world
            .handler
            .bot_checks
            .remaining_seconds(&(USER.to_owned(), "like_track".to_owned()))
            .is_some()
    );

    world.mobile.reply_with([Canned::Status(200)]);
    world.send("unlike_track", TRACK, None).await?;
    assert_eq!(world.mobile.count(), 2);
    assert_eq!(world.queued().await?, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_linked_comment_like_is_a_graph_upsert_and_an_unlike_is_a_graph_remove(
    pool: PgPool,
) -> anyhow::Result<()> {
    let world = world(pool, Profile::Active).await?;
    world.mobile.reply_with([
        Canned::Json(
            200,
            json!({"data": {"upsertInteraction": {"targetUrn": COMMENT}}}),
        ),
        Canned::Json(200, json!({"data": {"removeInteraction": true}})),
    ]);

    world
        .send("comment_like", COMMENT, comment_parent())
        .await?;
    world
        .send("comment_unlike", COMMENT, comment_parent())
        .await?;

    let bodies: Vec<Value> = (0..2)
        .filter_map(|index| world.mobile.sent(index))
        .filter_map(|(body, _)| serde_json::from_str(&body).ok())
        .collect();
    assert_eq!(bodies.len(), 2);
    assert!(
        bodies[0]["query"]
            .as_str()
            .is_some_and(|query| query.contains("upsertInteraction"))
    );
    assert!(
        bodies[1]["query"]
            .as_str()
            .is_some_and(|query| query.contains("removeInteraction"))
    );
    for body in &bodies {
        assert_eq!(
            body["variables"]["input"],
            json!({
                "targetUrn": COMMENT,
                "parentUrn": TRACK,
                "interactionTypeUrn": "sc:interactiontype:reaction",
                "interactionTypeValueUrn": "sc:interactiontypevalue:like",
            })
        );
    }
    assert!(world.apiv1.calls().is_empty());
    assert_eq!(world.queued().await?, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_comment_like_without_a_link_fails_with_its_reason_and_never_goes_to_apiv1(
    pool: PgPool,
) -> anyhow::Result<()> {
    let world = world(pool, Profile::Missing).await?;

    world
        .send("comment_like", COMMENT, comment_parent())
        .await?;

    assert_eq!(
        world.row().await?,
        (SYNC_MAX, true, Some("mobile_not_linked".to_owned()))
    );
    assert_eq!(world.mobile.count(), 0);
    assert!(world.apiv1.calls().is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_comment_like_without_a_track_parent_is_parked_before_anything_is_sent(
    pool: PgPool,
) -> anyhow::Result<()> {
    let world = world(pool, Profile::Active).await?;

    world.send("comment_like", COMMENT, None).await?;

    assert!(world.row().await?.1);
    assert_eq!(world.mobile.count(), 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_bot_check_on_a_comment_like_fails_it_without_apiv1(pool: PgPool) -> anyhow::Result<()> {
    let world = world(pool, Profile::Active).await?;
    world.mobile.reply_with([Canned::Captcha]);

    world
        .send("comment_like", COMMENT, comment_parent())
        .await?;

    assert_eq!(
        world.row().await?,
        (SYNC_MAX, true, Some("mobile_bot_check".to_owned()))
    );
    assert!(world.apiv1.calls().is_empty());
    Ok(())
}
