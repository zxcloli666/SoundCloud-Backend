use backend_contracts::remote::{LINK_CODE_TTL_SECS, LinkCodeResponse, LinkInfo, LinkProvider};
use sqlx::PgPool;
use uuid::Uuid;

use super::secrets::{
    code_hash, random_code, random_seed, random_token, rotated_tokens, token_hash,
};
use crate::error::{AppError, AppResult};

pub const AUTH_CODE_TTL_SECS: i32 = 300;
pub const ACCESS_TTL_SECS: i32 = 24 * 60 * 60;
pub const REFRESH_TTL_SECS: i32 = 365 * 24 * 60 * 60;
const CODE_ATTEMPTS: usize = 8;
pub const REFRESH_GRACE_SECS: i32 = 60;

pub struct LinksService {
    pg: PgPool,
}

#[derive(Debug, Clone)]
pub struct Grant {
    pub id: Uuid,
    pub sc_user_id: String,
    pub provider: LinkProvider,
}

#[derive(Debug, Clone)]
pub struct FoundCode {
    pub id: Uuid,
    pub user: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i32,
}

fn provider_of(raw: &str) -> AppResult<LinkProvider> {
    LinkProvider::parse(raw).ok_or_else(|| AppError::internal(format!("unknown provider {raw}")))
}

impl LinksService {
    pub fn new(pg: PgPool) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self { pg })
    }

    pub async fn create_code(
        &self,
        user: &str,
        provider: LinkProvider,
    ) -> AppResult<LinkCodeResponse> {
        sqlx::query_file!(
            "queries/remote/delete_stale_codes.sql",
            user,
            provider.as_str()
        )
        .execute(&self.pg)
        .await?;
        for _ in 0..CODE_ATTEMPTS {
            let code = random_code();
            let hash = code_hash(provider, &code);
            let taken = sqlx::query_file_scalar!("queries/remote/code_in_use.sql", hash)
                .fetch_one(&self.pg)
                .await?;
            if taken {
                continue;
            }
            let row = sqlx::query_file!(
                "queries/remote/insert_code.sql",
                Uuid::now_v7(),
                user,
                provider.as_str(),
                hash,
                LINK_CODE_TTL_SECS as i32
            )
            .fetch_one(&self.pg)
            .await?;
            return Ok(LinkCodeResponse {
                code,
                expires_at: row.expires_at.timestamp_millis(),
            });
        }
        Err(AppError::internal("no free link code"))
    }

    pub async fn find_code(
        &self,
        provider: LinkProvider,
        code: &str,
    ) -> AppResult<Option<FoundCode>> {
        let hash = code_hash(provider, code);
        Ok(
            sqlx::query_file!("queries/remote/find_code.sql", hash, provider.as_str())
                .fetch_optional(&self.pg)
                .await?
                .map(|row| FoundCode {
                    id: row.id,
                    user: row.sc_user_id,
                }),
        )
    }

    pub async fn consume_code(&self, id: Uuid) -> AppResult<bool> {
        Ok(sqlx::query_file!("queries/remote/consume_code.sql", id)
            .fetch_optional(&self.pg)
            .await?
            .is_some())
    }

    pub async fn issue_auth_code(
        &self,
        user: &str,
        provider: LinkProvider,
        client_id: &str,
        redirect_uri: &str,
    ) -> AppResult<String> {
        sqlx::query_file!("queries/remote/delete_stale_auth_codes.sql")
            .execute(&self.pg)
            .await?;
        let code = random_token();
        sqlx::query_file!(
            "queries/remote/insert_auth_code.sql",
            Uuid::now_v7(),
            user,
            provider.as_str(),
            client_id,
            token_hash(&code),
            redirect_uri,
            AUTH_CODE_TTL_SECS
        )
        .fetch_one(&self.pg)
        .await?;
        Ok(code)
    }

    pub async fn redeem_auth_code(
        &self,
        code: &str,
        client_id: &str,
        redirect_uri: &str,
    ) -> AppResult<Option<IssuedTokens>> {
        let access_token = random_token();
        let refresh_token = random_token();
        let row = sqlx::query_file!(
            "queries/remote/redeem_auth_code.sql",
            token_hash(code),
            token_hash(&access_token),
            ACCESS_TTL_SECS,
            token_hash(&refresh_token),
            REFRESH_TTL_SECS,
            client_id,
            redirect_uri
        )
        .fetch_optional(&self.pg)
        .await?;
        Ok(row.map(|_| IssuedTokens {
            access_token,
            refresh_token,
            expires_in: ACCESS_TTL_SECS,
        }))
    }

    pub async fn rotate_refresh(
        &self,
        refresh_token: &str,
        client_id: &str,
    ) -> AppResult<Option<IssuedTokens>> {
        let seed = random_seed();
        let (access_token, next_refresh) = rotated_tokens(&seed, refresh_token);
        let row = sqlx::query_file!(
            "queries/remote/rotate_refresh.sql",
            token_hash(refresh_token),
            token_hash(&access_token),
            ACCESS_TTL_SECS,
            token_hash(&next_refresh),
            REFRESH_TTL_SECS,
            client_id,
            seed
        )
        .fetch_optional(&self.pg)
        .await?;
        if row.is_some() {
            return Ok(Some(IssuedTokens {
                access_token,
                refresh_token: next_refresh,
                expires_in: ACCESS_TTL_SECS,
            }));
        }
        self.repeat_rotation(refresh_token, client_id).await
    }

    async fn repeat_rotation(
        &self,
        refresh_token: &str,
        client_id: &str,
    ) -> AppResult<Option<IssuedTokens>> {
        let Some(row) = sqlx::query_file!(
            "queries/remote/find_rotated.sql",
            token_hash(refresh_token),
            client_id,
            REFRESH_GRACE_SECS
        )
        .fetch_optional(&self.pg)
        .await?
        else {
            return Ok(None);
        };
        if !row.fresh {
            sqlx::query_file!("queries/remote/delete_grant.sql", row.id)
                .fetch_optional(&self.pg)
                .await?;
            tracing::warn!(grant = %row.id, "a rotated refresh token was used again, the grant is revoked");
            return Ok(None);
        }
        let (access_token, next_refresh) = rotated_tokens(&row.seed, refresh_token);
        if token_hash(&next_refresh) != row.refresh_hash {
            return Ok(None);
        }
        Ok(Some(IssuedTokens {
            access_token,
            refresh_token: next_refresh,
            expires_in: ACCESS_TTL_SECS,
        }))
    }

    pub async fn authenticate(&self, access_token: &str) -> AppResult<Option<Grant>> {
        let Some(row) = sqlx::query_file!(
            "queries/remote/find_by_access.sql",
            token_hash(access_token)
        )
        .fetch_optional(&self.pg)
        .await?
        else {
            return Ok(None);
        };
        sqlx::query_file!("queries/remote/touch_grant.sql", row.id)
            .execute(&self.pg)
            .await?;
        Ok(Some(Grant {
            id: row.id,
            sc_user_id: row.sc_user_id,
            provider: provider_of(&row.provider)?,
        }))
    }

    pub async fn bind_salut(&self, user: &str, sub: &str) -> AppResult<()> {
        sqlx::query_file!("queries/remote/delete_salut_binding.sql", sub)
            .execute(&self.pg)
            .await?;
        sqlx::query_file!(
            "queries/remote/insert_salut_grant.sql",
            Uuid::now_v7(),
            user,
            sub
        )
        .fetch_one(&self.pg)
        .await?;
        Ok(())
    }

    pub async fn find_salut(&self, sub: &str) -> AppResult<Option<Grant>> {
        let row = sqlx::query_file!("queries/remote/find_salut.sql", sub)
            .fetch_optional(&self.pg)
            .await?;
        row.map(|row| {
            Ok(Grant {
                id: row.id,
                sc_user_id: row.sc_user_id,
                provider: provider_of(&row.provider)?,
            })
        })
        .transpose()
    }

    pub async fn list(&self, user: &str) -> AppResult<Vec<LinkInfo>> {
        sqlx::query_file!("queries/remote/list_grants.sql", user)
            .fetch_all(&self.pg)
            .await?
            .into_iter()
            .map(|row| {
                Ok(LinkInfo {
                    id: row.id,
                    provider: provider_of(&row.provider)?,
                    created_at: row.created_at.timestamp_millis(),
                    last_used_at: row.last_used_at.map(|at| at.timestamp_millis()),
                })
            })
            .collect()
    }

    pub async fn revoke(&self, user: &str, id: Uuid) -> AppResult<bool> {
        Ok(
            sqlx::query_file!("queries/remote/revoke_grant.sql", id, user)
                .fetch_optional(&self.pg)
                .await?
                .is_some(),
        )
    }
}
