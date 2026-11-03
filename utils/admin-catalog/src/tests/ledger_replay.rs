use backend_contracts::admin_catalog::{AdminCommand, DetachArtist, SetPrimaryArtist};
use sqlx::PgPool;
use uuid::Uuid;

use super::support::{artist, credit, credits, send, send_as, track};

async fn ledger_rows(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM admin_catalog_requests")
        .fetch_one(pool)
        .await
        .unwrap()
}

fn remove_primary(track_id: Uuid, artist_id: Uuid) -> AdminCommand {
    AdminCommand::TrackRemoveCredit {
        track_id,
        artist_id,
        role: "primary".to_owned(),
    }
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn a_retried_request_replays_the_first_reply_and_applies_nothing_again(pool: PgPool) {
    let artist = artist(&pool, "Lead").await;
    let track = track(&pool, None).await;
    credit(&pool, track, artist, "primary").await;
    let request = Uuid::new_v4();

    let first = send_as(&pool, request, remove_primary(track, artist)).await;
    credit(&pool, track, artist, "primary").await;
    let retry = send_as(&pool, request, remove_primary(track, artist)).await;

    assert_eq!(first.body["removed"], 1);
    assert_eq!(retry, first);
    assert_eq!(
        credits(&pool, track).await,
        vec![(artist, "primary".to_owned())]
    );
    assert_eq!(ledger_rows(&pool).await, 1);
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn a_new_request_id_is_a_new_edit(pool: PgPool) {
    let artist = artist(&pool, "Lead").await;
    let track = track(&pool, None).await;
    credit(&pool, track, artist, "primary").await;

    let first = send(&pool, remove_primary(track, artist)).await;
    let second = send(&pool, remove_primary(track, artist)).await;

    assert_eq!(first.body["removed"], 1);
    assert_eq!(second.body["removed"], 0);
    assert_eq!(ledger_rows(&pool).await, 2);
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn one_request_id_cannot_carry_a_different_edit(pool: PgPool) {
    let artist = artist(&pool, "Lead").await;
    let track = track(&pool, None).await;
    let request = Uuid::new_v4();
    let set = AdminCommand::TrackSetPrimaryArtist {
        track_id: track,
        body: SetPrimaryArtist { artist_id: artist },
    };
    let detach = AdminCommand::TrackDetachArtist {
        track_id: track,
        body: DetachArtist {
            artist_id: artist,
            note: None,
        },
    };

    assert_eq!(send_as(&pool, request, set).await.status, 200);
    let reused = send_as(&pool, request, detach).await;

    assert_eq!(reused.status, 409);
    assert_eq!(
        credits(&pool, track).await,
        vec![(artist, "primary".to_owned())]
    );
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn a_refused_request_leaves_no_trace_and_can_be_sent_again(pool: PgPool) {
    let artist = artist(&pool, "Late").await;
    let track = track(&pool, None).await;
    let request = Uuid::new_v4();
    sqlx::query("DELETE FROM artists WHERE id = $1")
        .bind(artist)
        .execute(&pool)
        .await
        .unwrap();
    let set = AdminCommand::TrackSetPrimaryArtist {
        track_id: track,
        body: SetPrimaryArtist { artist_id: artist },
    };

    assert_eq!(send_as(&pool, request, set.clone()).await.status, 400);
    assert_eq!(ledger_rows(&pool).await, 0);

    sqlx::query("INSERT INTO artists (id, name, normalized_name, source) VALUES ($1, 'Late', 'late', 'genius')")
        .bind(artist)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(send_as(&pool, request, set).await.status, 200);
    assert_eq!(ledger_rows(&pool).await, 1);
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn two_copies_of_one_request_running_together_apply_once(pool: PgPool) {
    let artist = artist(&pool, "Lead").await;
    let track = track(&pool, None).await;
    credit(&pool, track, artist, "primary").await;
    let request = Uuid::new_v4();

    let (left, right) = tokio::join!(
        send_as(&pool, request, remove_primary(track, artist)),
        send_as(&pool, request, remove_primary(track, artist)),
    );

    assert_eq!(left.status, 200);
    assert_eq!(left, right);
    assert_eq!(left.body["removed"], 1);
    assert_eq!(ledger_rows(&pool).await, 1);
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn answers_older_than_a_week_are_pruned_by_the_next_edit(pool: PgPool) {
    let artist = artist(&pool, "Lead").await;
    let track = track(&pool, None).await;
    let old = Uuid::new_v4();
    send_as(&pool, old, remove_primary(track, artist)).await;
    sqlx::query("UPDATE admin_catalog_requests SET created_at = now() - interval '8 days'")
        .execute(&pool)
        .await
        .unwrap();

    send(&pool, remove_primary(track, artist)).await;

    let kept: Vec<Uuid> = sqlx::query_scalar("SELECT request_id FROM admin_catalog_requests")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(kept.len(), 1);
    assert!(!kept.contains(&old));
}
