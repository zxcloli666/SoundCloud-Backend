use std::sync::Arc;

use tokio::sync::RwLock;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::modules::auth::AuthService;
use crate::modules::oauth_apps::OAuthAppTokenService;
use crate::modules::oauth_apps::token_service::{PublicToken, PublicTokenId};
use crate::sc;

#[derive(Debug, Clone, Copy)]
pub enum TokenKind {
    UserFirst(Uuid),
    PublicPool,
}

#[derive(Clone)]
pub struct TokenChain {
    tokens: Arc<RwLock<Vec<ChainToken>>>,
    user: Option<UserToken>,
    app_tokens: Arc<OAuthAppTokenService>,
}

#[derive(Clone)]
struct UserToken {
    session_id: Uuid,
    auth: Arc<AuthService>,
}

#[derive(Clone)]
struct ChainToken {
    access_token: String,
    source: TokenSource,
}

#[derive(Clone, Copy)]
enum TokenSource {
    User,
    Public(PublicTokenId),
}

impl ChainToken {
    fn user(access_token: String) -> Self {
        Self {
            access_token,
            source: TokenSource::User,
        }
    }

    fn public(token: PublicToken) -> Self {
        let id = token.id();
        Self {
            access_token: token.access_token,
            source: TokenSource::Public(id),
        }
    }

    fn public_id(&self) -> Option<PublicTokenId> {
        match self.source {
            TokenSource::User => None,
            TokenSource::Public(id) => Some(id),
        }
    }

    fn is_user(&self) -> bool {
        matches!(self.source, TokenSource::User)
    }
}

pub struct TokenProvider {
    auth: Arc<AuthService>,
    app_tokens: Arc<OAuthAppTokenService>,
}

impl TokenProvider {
    pub fn new(auth: Arc<AuthService>, app_tokens: Arc<OAuthAppTokenService>) -> Arc<Self> {
        Arc::new(Self { auth, app_tokens })
    }

    pub async fn chain(&self, kind: TokenKind) -> AppResult<TokenChain> {
        match kind {
            TokenKind::UserFirst(session_id) => {
                let (user_token, public_tokens) = tokio::join!(
                    self.auth.get_immediately_usable_access_token(session_id),
                    self.app_tokens.snapshot(),
                );
                let public_pool_failed = public_tokens.is_err();
                let (tokens, has_user_token) = match user_first_tokens(user_token, public_tokens) {
                    Ok(tokens) => tokens,
                    Err(fast_error) => match self.auth.get_valid_access_token(session_id).await {
                        Ok(user_token) => (vec![ChainToken::user(user_token)], true),
                        Err(_) if public_pool_failed => return Err(fast_error),
                        Err(user_error) => return Err(user_error),
                    },
                };
                Ok(TokenChain {
                    tokens: Arc::new(RwLock::new(tokens)),
                    user: has_user_token.then(|| UserToken {
                        session_id,
                        auth: Arc::clone(&self.auth),
                    }),
                    app_tokens: Arc::clone(&self.app_tokens),
                })
            }
            TokenKind::PublicPool => {
                let tokens = self.app_tokens.snapshot().await?;
                Ok(TokenChain {
                    tokens: Arc::new(RwLock::new(
                        tokens.into_iter().map(ChainToken::public).collect(),
                    )),
                    user: None,
                    app_tokens: Arc::clone(&self.app_tokens),
                })
            }
        }
    }
}

fn user_first_tokens(
    user_token: AppResult<String>,
    public_tokens: AppResult<Vec<PublicToken>>,
) -> AppResult<(Vec<ChainToken>, bool)> {
    match (user_token, public_tokens) {
        (Ok(user_token), Ok(public_tokens)) => {
            let mut tokens = vec![ChainToken::user(user_token)];
            for token in public_tokens {
                if !tokens
                    .iter()
                    .any(|candidate| candidate.access_token == token.access_token)
                {
                    tokens.push(ChainToken::public(token));
                }
            }
            Ok((tokens, true))
        }
        (Ok(user_token), Err(_)) => Ok((vec![ChainToken::user(user_token)], true)),
        (Err(_), Ok(public_tokens)) if !public_tokens.is_empty() => Ok((
            public_tokens.into_iter().map(ChainToken::public).collect(),
            false,
        )),
        (Err(user_error), Ok(_)) => Err(user_error),
        (Err(_), Err(public_error)) => Err(public_error),
    }
}

pub async fn try_with_chain<F, Fut, T, E>(chain: &TokenChain, op: F) -> AppResult<T>
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: Into<AppError>,
{
    let mut last_error = None;
    let mut user_error = None;
    let tokens = chain.tokens.read().await.clone();
    for (index, token) in tokens.into_iter().enumerate() {
        match op(token.access_token.clone()).await.map_err(Into::into) {
            Ok(value) => return Ok(value),
            Err(error) if index == 0 && token.is_user() && token_rejected(&error) => {
                if let Some(user) = chain.user.as_ref() {
                    match user
                        .auth
                        .refresh_rejected_access_token(user.session_id, &token.access_token)
                        .await
                    {
                        Ok(refreshed_token) => {
                            replace_rejected_token(
                                &chain.tokens,
                                &token.access_token,
                                &refreshed_token,
                            )
                            .await;
                            match op(refreshed_token.clone()).await.map_err(Into::into) {
                                Ok(value) => return Ok(value),
                                Err(error) if token_rejected(&error) => {
                                    let retry_after = user
                                        .auth
                                        .mark_access_token_rejected(
                                            user.session_id,
                                            &refreshed_token,
                                        )
                                        .await?;
                                    user_error =
                                        Some(AppError::soundcloud_temporarily_unavailable_for(
                                            retry_after,
                                        ));
                                }
                                Err(error) if should_rotate(&error) => user_error = Some(error),
                                Err(error) => return Err(error),
                            }
                        }
                        Err(error) => user_error = Some(error),
                    }
                    continue;
                }
                last_error = Some(error);
            }
            Err(error) if token_rejected(&error) => {
                if let Some(public_id) = token.public_id() {
                    chain.app_tokens.reject(public_id).await;
                }
                last_error = Some(error);
            }
            Err(error) if should_rotate(&error) => {
                if index == 0 && chain.user.is_some() {
                    user_error = Some(error);
                } else {
                    last_error = Some(error);
                }
            }
            Err(error) => return Err(error),
        }
    }
    Err(terminal_chain_error(user_error, last_error))
}

async fn replace_rejected_token(tokens: &RwLock<Vec<ChainToken>>, rejected: &str, refreshed: &str) {
    let mut tokens = tokens.write().await;
    if tokens
        .first()
        .is_none_or(|current| !current.is_user() || current.access_token != rejected)
    {
        return;
    }
    let mut updated = Vec::with_capacity(tokens.len());
    updated.push(ChainToken::user(refreshed.to_owned()));
    updated.extend(
        tokens
            .iter()
            .skip(1)
            .filter(|token| token.access_token != refreshed)
            .cloned(),
    );
    *tokens = updated;
}

pub fn should_rotate(error: &AppError) -> bool {
    token_rejected(error)
        || sc::is_rate_limited(error)
        || sc::is_ban_error(error)
        || sc::is_upstream_failure(error)
        || unusable_body(error)
}

fn unusable_body(error: &AppError) -> bool {
    matches!(error, AppError::ScUnreachable(_))
}

fn token_rejected(error: &AppError) -> bool {
    matches!(error, AppError::ScApi { status: 401, .. })
}

fn terminal_chain_error(user_error: Option<AppError>, last_error: Option<AppError>) -> AppError {
    let error = user_error
        .or(last_error)
        .unwrap_or_else(|| AppError::internal("no tokens worked"));
    if token_rejected(&error) {
        AppError::soundcloud_temporarily_unavailable()
    } else {
        error
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_upstream_unauthorized_rejects_a_token() {
        assert!(token_rejected(&AppError::ScApi {
            status: 401,
            body: serde_json::Value::Null,
            retry_after_sec: None,
        }));
        assert!(!token_rejected(&AppError::ScApi {
            status: 403,
            body: serde_json::Value::Null,
            retry_after_sec: None,
        }));
    }

    #[test]
    fn a_broken_upstream_moves_the_chain_on_instead_of_burning_one_token() {
        for status in [500_u16, 502, 503] {
            assert!(
                should_rotate(&AppError::ScApi {
                    status,
                    body: serde_json::Value::Null,
                    retry_after_sec: None,
                }),
                "{status}"
            );
        }
        assert!(!should_rotate(&AppError::ScApi {
            status: 404,
            body: serde_json::Value::Null,
            retry_after_sec: None,
        }));
    }

    #[test]
    fn user_error_wins_over_a_public_token_rejection() {
        let error = terminal_chain_error(
            Some(AppError::soundcloud_reauthorization_required()),
            Some(AppError::ScApi {
                status: 401,
                body: serde_json::Value::Null,
                retry_after_sec: None,
            }),
        );

        assert!(matches!(error, AppError::SoundCloudReauthorizationRequired));
    }

    #[test]
    fn public_token_rejection_is_not_an_application_session_rejection() {
        let error = terminal_chain_error(
            None,
            Some(AppError::ScApi {
                status: 401,
                body: serde_json::Value::Null,
                retry_after_sec: None,
            }),
        );

        assert!(matches!(
            error,
            AppError::SoundCloudTemporarilyUnavailable { .. }
        ));
    }

    #[test]
    fn user_first_preserves_public_pool_failure_when_no_user_token_exists() {
        let tokens = user_first_tokens(
            Err(AppError::soundcloud_reauthorization_required()),
            Err(AppError::soundcloud_temporarily_unavailable()),
        );

        assert!(matches!(
            tokens,
            Err(AppError::SoundCloudTemporarilyUnavailable { .. })
        ));
    }

    #[test]
    fn user_first_can_serve_with_either_available_source() {
        let user_only = user_first_tokens(
            Ok("user".to_owned()),
            Err(AppError::soundcloud_temporarily_unavailable()),
        );
        let public_only = user_first_tokens(
            Err(AppError::soundcloud_reauthorization_required()),
            Ok(vec![public_token("public")]),
        );

        assert!(user_only.is_ok_and(|(tokens, has_user)| {
            has_user && tokens.len() == 1 && tokens[0].access_token == "user"
        }));
        assert!(public_only.is_ok_and(|(tokens, has_user)| {
            !has_user && tokens.len() == 1 && tokens[0].access_token == "public"
        }));
    }

    #[tokio::test]
    async fn refreshed_user_token_replaces_the_rejected_token() {
        let tokens = RwLock::new(vec![
            ChainToken::user("rejected".to_owned()),
            ChainToken::public(public_token("refreshed")),
            ChainToken::public(public_token("fallback")),
        ]);

        replace_rejected_token(&tokens, "rejected", "refreshed").await;

        assert_eq!(
            tokens
                .read()
                .await
                .iter()
                .map(|token| token.access_token.as_str())
                .collect::<Vec<_>>(),
            vec!["refreshed", "fallback"]
        );
    }

    #[sqlx::test(migrations = false)]
    async fn public_unauthorized_marks_the_observed_token_due(
        pool: sqlx::PgPool,
    ) -> anyhow::Result<()> {
        install_public_token_schema(&pool).await?;
        let token = public_token("rejected");
        insert_public_token(&pool, &token).await?;
        let chain = public_chain(pool.clone(), token);

        let result = try_with_chain(&chain, |_| async {
            Err::<(), _>(AppError::ScApi {
                status: 401,
                body: serde_json::Value::Null,
                retry_after_sec: None,
            })
        })
        .await;

        assert!(matches!(
            result,
            Err(AppError::SoundCloudTemporarilyUnavailable { .. })
        ));
        assert!(public_token_is_due(&pool).await?);
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn public_rate_limit_rotates_without_invalidating_token(
        pool: sqlx::PgPool,
    ) -> anyhow::Result<()> {
        install_public_token_schema(&pool).await?;
        let token = public_token("rate-limited");
        insert_public_token(&pool, &token).await?;
        let chain = public_chain(pool.clone(), token);

        let result = try_with_chain(&chain, |_| async {
            Err::<(), _>(AppError::ScApi {
                status: 429,
                body: serde_json::Value::Null,
                retry_after_sec: Some(5),
            })
        })
        .await;

        assert!(matches!(result, Err(AppError::ScApi { status: 429, .. })));
        assert!(!public_token_is_due(&pool).await?);
        Ok(())
    }

    async fn install_public_token_schema(pool: &sqlx::PgPool) -> anyhow::Result<()> {
        sqlx::query(
            "CREATE TABLE oauth_app_tokens (
                 oauth_app_id uuid PRIMARY KEY,
                 generation uuid NOT NULL,
                 expires_at timestamptz NOT NULL
             )",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn insert_public_token(pool: &sqlx::PgPool, token: &PublicToken) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO oauth_app_tokens (oauth_app_id, generation, expires_at)
             VALUES ($1, $2, $3)",
        )
        .bind(token.oauth_app_id)
        .bind(token.generation)
        .bind(token.expires_at)
        .execute(pool)
        .await?;
        Ok(())
    }

    fn public_chain(pool: sqlx::PgPool, token: PublicToken) -> TokenChain {
        TokenChain {
            tokens: Arc::new(RwLock::new(vec![ChainToken::public(token)])),
            user: None,
            app_tokens: OAuthAppTokenService::new(pool),
        }
    }

    async fn public_token_is_due(pool: &sqlx::PgPool) -> anyhow::Result<bool> {
        Ok(
            sqlx::query_scalar("SELECT expires_at <= now() FROM oauth_app_tokens")
                .fetch_one(pool)
                .await?,
        )
    }

    fn public_token(access_token: &str) -> PublicToken {
        PublicToken {
            oauth_app_id: Uuid::from_u128(1),
            generation: Uuid::from_u128(2),
            access_token: access_token.to_owned(),
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
        }
    }
}
