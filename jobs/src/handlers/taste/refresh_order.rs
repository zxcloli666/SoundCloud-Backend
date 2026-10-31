use chrono::NaiveDateTime;
use sqlx::PgPool;

use super::is_canonical_user;

pub(super) struct RefreshBatch {
    pub(super) users: Vec<String>,
    pub(super) star: usize,
}

impl RefreshBatch {
    pub(super) fn regular(&self) -> usize {
        self.users.len() - self.star
    }
}

pub(super) async fn users(
    pool: &PgPool,
    from: NaiveDateTime,
    until: NaiveDateTime,
) -> Result<(Vec<String>, Vec<String>), sqlx::Error> {
    let (star, regular): (Vec<_>, Vec<_>) =
        sqlx::query_file!("queries/taste/refresh_users.sql", from, until)
            .fetch_all(pool)
            .await?
            .into_iter()
            .filter(|row| is_canonical_user(&row.user_id))
            .partition(|row| row.star);
    Ok((
        star.into_iter().map(|row| row.user_id).collect(),
        regular.into_iter().map(|row| row.user_id).collect(),
    ))
}

pub(super) fn star_first(
    star: Vec<String>,
    regular: Vec<String>,
    size: usize,
) -> Vec<RefreshBatch> {
    let star_share = size - size / 4;
    let mut star = star.into_iter().peekable();
    let mut regular = regular.into_iter().peekable();
    let mut batches = Vec::new();
    while star.peek().is_some() || regular.peek().is_some() {
        let mut users: Vec<String> = star.by_ref().take(star_share).collect();
        let leading = users.len();
        users.extend(regular.by_ref().take(size - leading));
        let room = size - users.len();
        let trailing: Vec<String> = star.by_ref().take(room).collect();
        users.extend(trailing.iter().cloned());
        batches.push(RefreshBatch {
            users,
            star: leading + trailing.len(),
        });
    }
    batches
}

#[cfg(test)]
mod tests {
    use super::*;

    #[sqlx::test(migrations = "../api/migrations")]
    async fn users_with_new_events_are_split_by_a_live_star_subscription(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        sqlx::raw_sql(
            "INSERT INTO subscriptions (user_urn, exp_date) VALUES
                 ('7', extract(epoch FROM now())::bigint + 3600),
                 ('soundcloud:users:9', extract(epoch FROM now())::bigint + 3600),
                 ('8', extract(epoch FROM now())::bigint - 3600);
             INSERT INTO user_events (sc_user_id, sc_track_id, event_type, weight, created_at) VALUES
                 ('soundcloud:users:7', '1', 'like', 1, now() AT TIME ZONE 'UTC'),
                 ('8', '1', 'like', 1, now() AT TIME ZONE 'UTC'),
                 ('9', '1', 'like', 1, now() AT TIME ZONE 'UTC'),
                 ('10', '1', 'like', 1, (now() AT TIME ZONE 'UTC') - interval '2 days');",
        )
        .execute(&pool)
        .await?;
        let until = chrono::Utc::now().naive_utc() + chrono::Duration::minutes(1);

        let (mut star, regular) = users(&pool, until - chrono::Duration::hours(1), until).await?;
        star.sort();

        assert_eq!(star, ["7", "9"]);
        assert_eq!(regular, ["8"]);
        Ok(())
    }

    #[sqlx::test(migrations = "../api/migrations")]
    async fn only_taste_event_types_ask_for_a_refresh(pool: PgPool) -> anyhow::Result<()> {
        sqlx::raw_sql(
            "INSERT INTO user_events (sc_user_id, sc_track_id, event_type, weight, created_at) VALUES
                 ('1', '1', 'skip', -0.8, now() AT TIME ZONE 'UTC'),
                 ('2', '1', 'skip', 0.0, now() AT TIME ZONE 'UTC'),
                 ('3', '1', 'dislike', -1.0, now() AT TIME ZONE 'UTC'),
                 ('4', '1', 'play_complete', 0.3, now() AT TIME ZONE 'UTC'),
                 ('5', '1', 'playlist_add', 0.9, now() AT TIME ZONE 'UTC'),
                 ('6', '1', 'full_play', 0.3, now() AT TIME ZONE 'UTC'),
                 ('soundcloud:users:7', '1', 'like', 1, now() AT TIME ZONE 'UTC');",
        )
        .execute(&pool)
        .await?;
        let until = chrono::Utc::now().naive_utc() + chrono::Duration::minutes(1);

        let (star, mut regular) = users(&pool, until - chrono::Duration::hours(1), until).await?;
        regular.sort();

        assert!(star.is_empty());
        assert_eq!(regular, ["1", "2", "3", "5", "6", "7"]);
        Ok(())
    }

    fn named(prefix: &str, count: usize) -> Vec<String> {
        (0..count).map(|index| format!("{prefix}{index}")).collect()
    }

    fn shape(batches: &[RefreshBatch]) -> Vec<(usize, usize)> {
        batches
            .iter()
            .map(|batch| (batch.star, batch.regular()))
            .collect()
    }

    #[test]
    fn star_users_lead_every_batch_and_regular_users_keep_a_quarter() {
        let batches = star_first(named("s", 10), named("r", 10), 4);

        assert_eq!(shape(&batches), [(3, 1), (3, 1), (3, 1), (1, 3), (0, 4)]);
        assert_eq!(batches[0].users, ["s0", "s1", "s2", "r0"]);
    }

    #[test]
    fn a_side_that_runs_out_leaves_its_room_to_the_other() {
        assert_eq!(
            shape(&star_first(named("s", 6), named("r", 1), 4)),
            [(3, 1), (3, 0)]
        );
        assert_eq!(
            shape(&star_first(Vec::new(), named("r", 5), 4)),
            [(0, 4), (0, 1)]
        );
        assert!(star_first(Vec::new(), Vec::new(), 4).is_empty());
    }

    #[test]
    fn every_user_is_refreshed_once() {
        let batches = star_first(named("s", 7), named("r", 9), 4);
        let mut users: Vec<String> = batches.into_iter().flat_map(|batch| batch.users).collect();
        users.sort();
        let mut expected = [named("s", 7), named("r", 9)].concat();
        expected.sort();
        assert_eq!(users, expected);
    }
}
