use backend_contracts::admin_catalog::{AdminCommand, ArtistCreate, ArtistUpdate};
use serde_json::json;
use sqlx::PgPool;

use super::support::{artist, credit, send, track};

fn create(name: &str) -> AdminCommand {
    AdminCommand::ArtistCreate(ArtistCreate {
        name: name.to_owned(),
        country: Some("DE".to_owned()),
        bio: None,
        avatar_url: None,
        sc_user_id: None,
    })
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn a_created_artist_is_manual_and_a_namesake_is_refused(pool: PgPool) {
    let created = send(&pool, create("  Kavinsky  ")).await;
    assert_eq!(created.status, 200);
    assert_eq!(created.body["name"], "Kavinsky");
    assert_eq!(created.body["source"], "manual");

    let again = send(&pool, create("kavinsky")).await;
    assert_eq!(again.status, 400);
    assert_eq!(
        again.body["message"],
        "artist with this name already exists"
    );
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn an_update_changes_only_the_given_fields(pool: PgPool) {
    let id = artist(&pool, "Old Name").await;
    let reply = send(
        &pool,
        AdminCommand::ArtistUpdate {
            artist_id: id,
            body: ArtistUpdate {
                name: Some("New Name".to_owned()),
                country: Some("FR".to_owned()),
                bio: None,
                avatar_url: None,
                sc_user_id: None,
                confidence: Some(0.5),
            },
        },
    )
    .await;
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body["name"], "New Name");
    assert_eq!(reply.body["normalized_name"], "new name");
    assert_eq!(reply.body["country"], "FR");
    assert_eq!(reply.body["confidence"], json!(0.5));
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn updating_a_missing_artist_is_not_found(pool: PgPool) {
    let reply = send(
        &pool,
        AdminCommand::ArtistUpdate {
            artist_id: uuid::Uuid::new_v4(),
            body: ArtistUpdate {
                name: None,
                country: None,
                bio: None,
                avatar_url: None,
                sc_user_id: None,
                confidence: None,
            },
        },
    )
    .await;
    assert_eq!(reply.status, 404);
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn a_merge_moves_credits_and_primary_artist_to_the_target(pool: PgPool) {
    let src = artist(&pool, "Source").await;
    let dst = artist(&pool, "Target").await;
    let track = track(&pool, None).await;
    credit(&pool, track, src, "primary").await;
    sqlx::query("UPDATE tracks SET primary_artist_id = $1 WHERE id = $2")
        .bind(src)
        .bind(track)
        .execute(&pool)
        .await
        .unwrap();

    let reply = send(
        &pool,
        AdminCommand::ArtistMerge {
            src_id: src,
            dst_id: dst,
        },
    )
    .await;

    assert_eq!(reply.status, 200);
    assert_eq!(reply.body["moved_track_artists"], 1);
    assert_eq!(
        super::support::credits(&pool, track).await,
        vec![(dst, "primary".to_owned())]
    );
    assert_eq!(super::support::primary(&pool, track).await, Some(dst));
    let merged_into: Option<uuid::Uuid> =
        sqlx::query_scalar("SELECT merged_into FROM artists WHERE id = $1")
            .bind(src)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(merged_into, Some(dst));
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn merging_an_artist_into_itself_is_refused(pool: PgPool) {
    let id = artist(&pool, "Same").await;
    let reply = send(
        &pool,
        AdminCommand::ArtistMerge {
            src_id: id,
            dst_id: id,
        },
    )
    .await;
    assert_eq!(reply.status, 400);
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn retrying_a_crawl_resets_the_failure_state(pool: PgPool) {
    let id = artist(&pool, "Crawled").await;
    sqlx::query("UPDATE artists SET crawl_dead = true, crawl_fail_count = 7 WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();

    let reply = send(&pool, AdminCommand::ArtistRetryCrawl { artist_id: id }).await;

    assert_eq!(reply.body["reset"], 1);
    let (dead, fails): (bool, i32) =
        sqlx::query_as("SELECT crawl_dead, crawl_fail_count::int FROM artists WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!dead);
    assert_eq!(fails, 0);
}
