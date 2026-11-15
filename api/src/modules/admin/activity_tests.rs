use chrono::{Duration, NaiveDateTime, Utc};
use sqlx::PgPool;

use super::*;

fn now() -> NaiveDateTime {
    Utc::now().naive_utc()
}

async fn play(pool: &PgPool, user: &str, days_ago: i64, hour: i64) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO listening_history (soundcloud_user_id, sc_track_id, title, artist_name, duration, played_at)
         VALUES ($1, 'soundcloud:tracks:1', 'Song', 'Artist', 1000, $2)",
    )
    .bind(user)
    .bind(now() - Duration::days(days_ago) - Duration::hours(hour))
    .execute(pool)
    .await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn active_users_get_28_day_aggregates(pool: PgPool) -> anyhow::Result<()> {
    for day in [1, 1, 2, 5, 27, 40, 90] {
        play(&pool, "101", day, 1).await?;
    }
    play(&pool, "202", 60, 0).await?;

    let since = now() - Duration::days(28);
    let page = page(&pool, since, "", 50).await?;

    assert_eq!(page.next, None);
    assert_eq!(page.users.len(), 1);
    let user = &page.users[0];
    assert_eq!(user.user_id, "101");
    assert_eq!(user.plays_28, 5);
    assert_eq!(user.active_days_28, 4);
    assert!(user.first_play_at < user.last_play_at);
    assert!(Utc::now().timestamp() - user.first_play_at >= 90 * 86_400);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn urn_and_bare_rows_count_as_one_user(pool: PgPool) -> anyhow::Result<()> {
    play(&pool, "303", 1, 0).await?;
    play(&pool, "soundcloud:users:303", 3, 0).await?;
    play(&pool, "soundcloud:users:303", 50, 0).await?;

    let page = page(&pool, now() - Duration::days(28), "", 50).await?;

    assert_eq!(page.users.len(), 1);
    assert_eq!(page.users[0].user_id, "303");
    assert_eq!(page.users[0].plays_28, 2);
    assert_eq!(page.users[0].active_days_28, 2);
    assert!(Utc::now().timestamp() - page.users[0].first_play_at >= 50 * 86_400);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn pages_walk_every_user_once(pool: PgPool) -> anyhow::Result<()> {
    for user in ["1", "2", "3", "4", "5"] {
        play(&pool, user, 1, 0).await?;
        play(&pool, user, 2, 0).await?;
    }
    play(&pool, "6", 100, 0).await?;

    let since = now() - Duration::days(28);
    let mut seen = Vec::new();
    let mut after = String::new();
    loop {
        let page = page(&pool, since, &after, 2).await?;
        seen.extend(page.users.into_iter().map(|u| u.user_id));
        match page.next {
            Some(next) => after = next,
            None => break,
        }
    }

    assert_eq!(seen, ["1", "2", "3", "4", "5"]);
    Ok(())
}
