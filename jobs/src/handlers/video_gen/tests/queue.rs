use sqlx::PgPool;

use super::super::state;
use super::{bench, config, flag, queued, star, superlike, track};

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn star_superlikes_own_tracks_and_flags_are_queued(pool: PgPool) {
    star(&pool, "soundcloud:users:11").await;
    star(&pool, "12").await;
    flag(&pool, "account", "13").await;
    flag(&pool, "artist", "14").await;
    for (id, uploader) in [
        ("501", "90"),
        ("502", "12"),
        ("503", "14"),
        ("504", "90"),
        ("505", "90"),
        ("506", "90"),
        ("507", "90"),
        ("508", "90"),
    ] {
        track(&pool, id, uploader).await;
    }
    superlike(&pool, "11", "501").await;
    superlike(&pool, "soundcloud:users:13", "504").await;
    superlike(&pool, "99", "505").await;
    superlike(&pool, "11", "506").await;
    superlike(&pool, "11", "507").await;
    superlike(&pool, "11", "508").await;
    sqlx::query("UPDATE tracks SET sharing = 'private' WHERE sc_track_id = '506'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO track_videos (sc_track_id, kind, source, s3_key, duration_ms)
         VALUES ('507', 'clip', 'youtube', 'video/507.mp4', 10000)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE tracks SET artwork_url = NULL WHERE sc_track_id = '508'")
        .execute(&pool)
        .await
        .unwrap();

    assert_eq!(state::enqueue(&pool, 10).await.unwrap(), 4);
    assert_eq!(queued(&pool).await, ["501", "502", "503", "504"]);
    assert_eq!(state::enqueue(&pool, 10).await.unwrap(), 0);
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_queue_stays_within_twice_the_in_flight_count(pool: PgPool) {
    star(&pool, "12").await;
    for id in ["501", "502", "503", "504", "505", "506"] {
        track(&pool, id, "12").await;
    }
    let mut one = config();
    one.in_flight = 1;
    let (bench, director, _, _, _) = bench(&pool, one);

    bench.dispatch().await.unwrap();

    assert_eq!(queued(&pool).await, ["501", "502"]);
    assert_eq!(director.briefs.lock().unwrap().len(), 1);
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn an_expired_star_queues_nothing(pool: PgPool) {
    sqlx::query("INSERT INTO subscriptions (user_urn, exp_date) VALUES ('12', extract(epoch FROM now())::bigint - 60)")
        .execute(&pool)
        .await
        .unwrap();
    track(&pool, "501", "12").await;

    assert_eq!(state::enqueue(&pool, 10).await.unwrap(), 0);
}
