use backend_contracts::admin_catalog::{DetachAccountTracks, SetPrimaryArtist};
use sqlx::PgPool;
use uuid::Uuid;

use super::support::{artist, credit, sc_track_id, track};
use crate::tracks;

const ROUNDS: usize = 40;

fn deadlock(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|database| database.code())
        .is_some_and(|code| code == "40P01")
}

async fn admin_edits(pool: PgPool, ids: Vec<Uuid>, artist_id: Uuid) -> usize {
    let mut deadlocks = 0;
    for round in 0..ROUNDS {
        let id = ids[round % ids.len()];
        let mut transaction = pool.begin().await.unwrap();
        let body = SetPrimaryArtist { artist_id };
        let outcome = tracks::set_primary_artist(&mut transaction, id, &body).await;
        match outcome {
            Ok(_) => transaction.commit().await.unwrap(),
            Err(crate::error::Fail::Database(error)) if deadlock(&error) => deadlocks += 1,
            Err(_) => panic!("admin edit failed"),
        }
    }
    deadlocks
}

async fn admin_detaches(pool: PgPool, artist_id: Uuid, uploader: String) -> usize {
    let mut deadlocks = 0;
    for _ in 0..ROUNDS {
        let mut transaction = pool.begin().await.unwrap();
        let body = DetachAccountTracks { note: None };
        let outcome =
            tracks::detach_account_tracks(&mut transaction, artist_id, &uploader, &body).await;
        match outcome {
            Ok(_) => transaction.commit().await.unwrap(),
            Err(crate::error::Fail::Database(error)) if deadlock(&error) => deadlocks += 1,
            Err(_) => panic!("detach failed"),
        }
    }
    deadlocks
}

async fn persist_like_batches(pool: PgPool, ids: Vec<Uuid>) -> usize {
    let mut deadlocks = 0;
    for round in 0..ROUNDS {
        let mut transaction = pool.begin().await.unwrap();
        let credits = sqlx::query(
            "WITH locked AS MATERIALIZED (SELECT credit.track_id, credit.artist_id, credit.role
             FROM track_artists credit WHERE credit.track_id = ANY($1)
             ORDER BY credit.track_id, credit.artist_id, credit.role FOR UPDATE OF credit)
             DELETE FROM track_artists ta USING locked
             WHERE ta.track_id = locked.track_id AND ta.artist_id = locked.artist_id AND ta.role = locked.role",
        )
        .bind(&ids)
        .execute(&mut *transaction)
        .await;
        let result = match credits {
            Ok(_) => {
                let locked: Result<Vec<Uuid>, sqlx::Error> = sqlx::query_scalar(
                    "SELECT track.id FROM tracks track WHERE track.id = ANY($1)
                     ORDER BY track.sc_track_id FOR NO KEY UPDATE OF track",
                )
                .bind(&ids)
                .fetch_all(&mut *transaction)
                .await;
                match locked {
                    Ok(locked) => {
                        sqlx::query(
                            "UPDATE tracks SET title = 'batch ' || $2::text WHERE id = ANY($1)",
                        )
                        .bind(&locked)
                        .bind(round as i32)
                        .execute(&mut *transaction)
                        .await
                    }
                    Err(error) => Err(error),
                }
            }
            Err(error) => Err(error),
        };
        match result {
            Ok(_) => transaction.commit().await.unwrap(),
            Err(error) if deadlock(&error) => deadlocks += 1,
            Err(error) => panic!("batch failed: {error}"),
        }
    }
    deadlocks
}

#[sqlx::test(migrations = "../../api/migrations")]
async fn admin_edits_and_jobs_batches_on_the_same_tracks_never_deadlock(pool: PgPool) {
    let uploader = "9000001";
    let artist_id = artist(&pool, "Lock Order").await;
    let mut ids = Vec::new();
    for _ in 0..6 {
        let id = track(&pool, Some(uploader)).await;
        credit(&pool, id, artist_id, "primary").await;
        ids.push(id);
    }
    let mut by_key = Vec::new();
    for id in &ids {
        by_key.push((sc_track_id(&pool, *id).await, *id));
    }
    by_key.sort();
    let sorted: Vec<Uuid> = by_key.iter().map(|(_, id)| *id).collect();
    let mut reversed = sorted.clone();
    reversed.reverse();

    let tasks = vec![
        tokio::spawn(admin_edits(pool.clone(), sorted.clone(), artist_id)),
        tokio::spawn(admin_edits(pool.clone(), reversed.clone(), artist_id)),
        tokio::spawn(admin_detaches(pool.clone(), artist_id, uploader.to_owned())),
        tokio::spawn(persist_like_batches(pool.clone(), sorted)),
        tokio::spawn(persist_like_batches(pool.clone(), reversed)),
    ];
    let mut deadlocks = 0;
    for task in tasks {
        deadlocks += task.await.unwrap();
    }
    assert_eq!(deadlocks, 0);
}
