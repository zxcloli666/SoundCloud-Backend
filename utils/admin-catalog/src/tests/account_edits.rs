use backend_contracts::admin_catalog::{AdminCommand, ScAccountUpsert};
use sqlx::PgPool;
use uuid::Uuid;

use super::support::{artist, send};

fn upsert(artist_id: Uuid, sc_user_id: &str, role: &str, notes: Option<&str>) -> AdminCommand {
    AdminCommand::ScAccountUpsert {
        artist_id,
        body: ScAccountUpsert {
            sc_user_id: sc_user_id.to_owned(),
            role: role.to_owned(),
            notes: notes.map(str::to_owned),
        },
    }
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn a_manual_account_is_verified_and_keeps_its_notes(pool: PgPool) {
    let artist = artist(&pool, "Owner").await;

    let reply = send(&pool, upsert(artist, "204411307", "main", Some("official"))).await;

    assert_eq!(reply.status, 200);
    let (verified, source, notes): (bool, String, Option<String>) = sqlx::query_as(
        "SELECT verified, source::text, notes FROM artist_sc_accounts WHERE artist_id = $1",
    )
    .bind(artist)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(verified);
    assert_eq!(source, "manual");
    assert_eq!(notes.as_deref(), Some("official"));
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn a_verified_account_cannot_belong_to_two_artists(pool: PgPool) {
    let first = artist(&pool, "First Owner").await;
    let second = artist(&pool, "Second Owner").await;
    send(&pool, upsert(first, "204411307", "main", None)).await;

    let reply = send(&pool, upsert(second, "204411307", "main", None)).await;

    assert_eq!(reply.status, 409);
    let owners: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM artist_sc_accounts WHERE sc_user_id = '204411307'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(owners, 1);
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn an_account_needs_a_known_role_and_an_id(pool: PgPool) {
    let artist = artist(&pool, "Strict").await;

    assert_eq!(
        send(&pool, upsert(artist, "1", "boss", None)).await.status,
        400
    );
    assert_eq!(
        send(&pool, upsert(artist, "  ", "main", None)).await.status,
        400
    );
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn deleting_an_account_reports_whether_it_existed(pool: PgPool) {
    let artist = artist(&pool, "Leaver").await;
    send(&pool, upsert(artist, "555", "alt", None)).await;
    let delete = || AdminCommand::ScAccountDelete {
        artist_id: artist,
        sc_user_id: "555".to_owned(),
    };

    assert_eq!(send(&pool, delete()).await.body["ok"], true);
    assert_eq!(send(&pool, delete()).await.body["ok"], false);
}
