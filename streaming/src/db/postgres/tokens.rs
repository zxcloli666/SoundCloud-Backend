#[cfg(test)]
#[path = "tokens_tests.rs"]
mod tests;

use rand::seq::SliceRandom;

use super::{PgError, PgPool};

const APP_TOKENS: &str = r#"SELECT token.access_token
   FROM (SELECT oauth_app_id, access_token, expires_at FROM oauth_app_tokens
         UNION
         SELECT oauth_app_id, access_token, expires_at FROM oauth_app_tokens_local) AS token
   JOIN oauth_apps AS app ON app.id = token.oauth_app_id
   WHERE app.active
     AND token.expires_at > NOW() + INTERVAL '30 seconds'
     AND token.access_token <> ''
     AND token.access_token <> $1"#;

impl PgPool {
    pub async fn get_app_tokens(&self, exclude_token: &str) -> Result<Vec<String>, PgError> {
        let client = self.connection().await?;
        let rows = client.query(APP_TOKENS, &[&exclude_token]).await?;
        let mut tokens: Vec<String> = rows.iter().map(|r| r.get(0)).collect();
        tokens.shuffle(&mut rand::thread_rng());
        Ok(tokens)
    }
}
