use backend_contracts::admin_catalog::{AdminCommand, CreditRef};
use sqlx::PgPool;

use super::support::{artist, credit, credits, primary, send, track};

async fn weak_primary(pool: &PgPool) -> (uuid::Uuid, uuid::Uuid, CreditRef) {
    let artist = artist(pool, "Guess").await;
    let track = track(pool, None).await;
    credit(pool, track, artist, "primary").await;
    sqlx::query("UPDATE tracks SET primary_artist_id = $1, enrich_state = 'done' WHERE id = $2")
        .bind(artist)
        .bind(track)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE track_artists SET evidence = 'title_heuristic' WHERE track_id = $1")
        .bind(track)
        .execute(pool)
        .await
        .unwrap();
    let reference = CreditRef {
        track_id: track,
        artist_id: artist,
        role: "primary".to_owned(),
    };
    (track, artist, reference)
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn accepting_a_weak_credit_makes_it_manual_evidence(pool: PgPool) {
    let (track, artist, reference) = weak_primary(&pool).await;

    let reply = send(&pool, AdminCommand::CreditAccept(reference.clone())).await;

    assert_eq!(reply.status, 200);
    assert_eq!(reply.body["applied"], true);
    assert_eq!(reply.body["primaryCleared"], false);
    let evidence: String =
        sqlx::query_scalar("SELECT evidence FROM track_artists WHERE track_id = $1")
            .bind(track)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(evidence, "manual");
    assert_eq!(primary(&pool, track).await, Some(artist));

    let again = send(&pool, AdminCommand::CreditAccept(reference)).await;
    assert_eq!(again.status, 404);
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn rejecting_a_weak_primary_credit_clears_the_primary_artist(pool: PgPool) {
    let (track, _, reference) = weak_primary(&pool).await;

    let reply = send(&pool, AdminCommand::CreditReject(reference.clone())).await;

    assert_eq!(reply.body["applied"], true);
    assert_eq!(reply.body["primaryCleared"], true);
    assert_eq!(primary(&pool, track).await, None);
    assert!(credits(&pool, track).await.is_empty());

    let again = send(&pool, AdminCommand::CreditReject(reference)).await;
    assert_eq!(again.status, 404);
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn a_credit_with_real_evidence_is_not_a_weak_credit(pool: PgPool) {
    let artist = artist(&pool, "Verified").await;
    let track = track(&pool, None).await;
    credit(&pool, track, artist, "primary").await;
    sqlx::query("UPDATE track_artists SET evidence = 'external_id' WHERE track_id = $1")
        .bind(track)
        .execute(&pool)
        .await
        .unwrap();

    let reply = send(
        &pool,
        AdminCommand::CreditReject(CreditRef {
            track_id: track,
            artist_id: artist,
            role: "primary".to_owned(),
        }),
    )
    .await;

    assert_eq!(reply.status, 404);
    assert_eq!(credits(&pool, track).await.len(), 1);
}
