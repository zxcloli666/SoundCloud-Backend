use sqlx::PgPool;

use crate::error::AppResult;
use crate::modules::rooms::model::Profile;

fn stored(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
}

pub async fn account_profile(
    pg: &PgPool,
    sc_user_id: &str,
    claimed: Profile,
) -> AppResult<Profile> {
    let account = sqlx::query_file!("queries/rooms/account_profile.sql", sc_user_id)
        .fetch_one(pg)
        .await?;
    Ok(Profile {
        name: stored(account.username).unwrap_or(claimed.name),
        avatar_url: stored(account.avatar_url).or(claimed.avatar_url),
    })
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
