use std::sync::Arc;

use sqlx::PgPool;
use tracing::info;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::modules::oauth_apps::dto::{CreateOAuthAppDto, UpdateOAuthAppDto};
use crate::modules::oauth_apps::model::{ClientKind, OAuthApp};

const MOBILE_REDIRECT_URI: &str = "sc://auth";

pub struct OAuthAppsService {
    pool: PgPool,
}

impl OAuthAppsService {
    pub fn new(pool: PgPool) -> Arc<Self> {
        Arc::new(Self { pool })
    }

    pub async fn count_active(&self) -> AppResult<i64> {
        let n: i64 = sqlx::query_file_scalar!("queries/oauth_apps/service/count_active.sql")
            .fetch_one(&self.pool)
            .await?;
        Ok(n)
    }

    pub async fn pick_lru_from(&self, ids: &[Uuid]) -> AppResult<OAuthApp> {
        if ids.is_empty() {
            return Err(AppError::service_unavailable(
                "No active OAuth apps available",
            ));
        }
        let app = sqlx::query_file_as!(
            OAuthApp,
            "queries/oauth_apps/service/pick_lru_from.sql",
            ids
        )
        .fetch_optional(&self.pool)
        .await?;
        let app = match app {
            Some(app) => Some(app),
            None => {
                sqlx::query_file_as!(
                    OAuthApp,
                    "queries/oauth_apps/service/pick_lru_wait.sql",
                    ids
                )
                .fetch_optional(&self.pool)
                .await?
            }
        };

        let app =
            app.ok_or_else(|| AppError::service_unavailable("No active OAuth apps available"))?;
        info!(app_name = %app.name, app_id = %app.id, "Picked OAuth app — lastUsedAt updated");
        Ok(app)
    }

    pub async fn get_by_id(&self, id: &str) -> AppResult<Option<OAuthApp>> {
        let uuid = match Uuid::parse_str(id) {
            Ok(u) => u,
            Err(_) => return Ok(None),
        };
        let row: Option<OAuthApp> =
            sqlx::query_file_as!(OAuthApp, "queries/oauth_apps/service/get_by_id.sql", uuid)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row)
    }

    pub async fn find_all(&self) -> AppResult<Vec<OAuthApp>> {
        let rows: Vec<OAuthApp> =
            sqlx::query_file_as!(OAuthApp, "queries/oauth_apps/service/find_all.sql")
                .fetch_all(&self.pool)
                .await?;
        Ok(rows)
    }

    pub async fn create(&self, dto: &CreateOAuthAppDto) -> AppResult<OAuthApp> {
        let kind = dto.client_kind;
        let name = normalized_non_blank("name", &dto.name)?;
        let client_id = normalized_non_blank("clientId", &dto.client_id)?;
        let client_secret = non_blank_secret(&dto.client_secret)?;
        let redirect_uri = normalized_redirect_uri(&dto.redirect_uri, kind)?;
        let app_id = profile_field(kind, "appId", dto.app_id.as_deref())?;
        let app_version = profile_field(kind, "appVersion", dto.app_version.as_deref())?;
        let user_agent = profile_field(kind, "userAgent", dto.user_agent.as_deref())?;
        let row: OAuthApp = sqlx::query_file_as!(
            OAuthApp,
            "queries/oauth_apps/service/create.sql",
            Uuid::now_v7(),
            &name,
            &client_id,
            client_secret,
            &redirect_uri,
            dto.active.unwrap_or(true),
            kind.as_str(),
            app_id,
            app_version,
            user_agent
        )
        .fetch_one(&self.pool)
        .await
        .map_err(oauth_app_write_error)?;
        Ok(row)
    }

    pub async fn update(&self, id: &str, dto: &UpdateOAuthAppDto) -> AppResult<OAuthApp> {
        let uuid = Uuid::parse_str(id).map_err(|_| AppError::not_found("OAuth app not found"))?;
        let current = self
            .get_by_id(id)
            .await?
            .ok_or_else(|| AppError::not_found("OAuth app not found"))?;
        if dto
            .client_id
            .as_deref()
            .is_some_and(|value| value.trim() != current.client_id.trim())
        {
            return Err(AppError::bad_request(
                "clientId is immutable; create a new OAuth app",
            ));
        }
        if dto
            .client_kind
            .is_some_and(|kind| kind.as_str() != current.client_kind)
        {
            return Err(AppError::bad_request(
                "clientKind is immutable; create a new OAuth app",
            ));
        }
        let kind = if current.client_kind == ClientKind::Mobile.as_str() {
            ClientKind::Mobile
        } else {
            ClientKind::Default
        };
        let name = dto
            .name
            .as_deref()
            .map(|value| normalized_non_blank("name", value))
            .transpose()?;
        let client_secret = dto
            .client_secret
            .as_deref()
            .map(non_blank_secret)
            .transpose()?;
        let redirect_uri = dto
            .redirect_uri
            .as_deref()
            .map(|value| normalized_redirect_uri(value, kind))
            .transpose()?;
        let app_id = dto
            .app_id
            .as_deref()
            .map(|value| normalized_non_blank("appId", value))
            .transpose()?;
        let app_version = dto
            .app_version
            .as_deref()
            .map(|value| normalized_non_blank("appVersion", value))
            .transpose()?;
        let user_agent = dto
            .user_agent
            .as_deref()
            .map(|value| normalized_non_blank("userAgent", value))
            .transpose()?;
        let row: Option<OAuthApp> = sqlx::query_file_as!(
            OAuthApp,
            "queries/oauth_apps/service/update.sql",
            uuid,
            name.as_deref(),
            client_secret,
            redirect_uri.as_deref(),
            dto.active,
            app_id.as_deref(),
            app_version.as_deref(),
            user_agent.as_deref()
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(oauth_app_write_error)?;
        row.ok_or_else(|| AppError::not_found("OAuth app not found"))
    }

    pub async fn remove(&self, id: &str) -> AppResult<()> {
        let uuid = match Uuid::parse_str(id) {
            Ok(u) => u,
            Err(_) => return Ok(()),
        };
        let mut transaction = self.pool.begin().await?;
        wt::call(&mut transaction, &wt::OauthAppDelete { app_id: uuid }).await?;
        transaction.commit().await?;
        Ok(())
    }
}

fn normalized_non_blank(field: &str, value: &str) -> AppResult<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AppError::bad_request(format!("{field} must not be blank")));
    }
    Ok(value.to_owned())
}

fn non_blank_secret(value: &str) -> AppResult<&str> {
    if value.trim().is_empty() {
        return Err(AppError::bad_request("clientSecret must not be blank"));
    }
    Ok(value)
}

fn normalized_redirect_uri(value: &str, kind: ClientKind) -> AppResult<String> {
    let value = normalized_non_blank("redirectUri", value)?;
    let uri = url::Url::parse(&value)
        .map_err(|_| AppError::bad_request("redirectUri must be a valid URL"))?;
    let allowed = match uri.scheme() {
        "http" | "https" => true,
        "sc" => kind == ClientKind::Mobile && value == MOBILE_REDIRECT_URI,
        _ => false,
    };
    if !allowed {
        return Err(AppError::bad_request("redirectUri must use HTTP or HTTPS"));
    }
    Ok(value)
}

fn profile_field(kind: ClientKind, field: &str, value: Option<&str>) -> AppResult<Option<String>> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => Ok(Some(value.to_owned())),
        None if kind == ClientKind::Mobile => Err(AppError::bad_request(format!(
            "{field} is required for a mobile app"
        ))),
        None => Ok(None),
    }
}

fn oauth_app_write_error(error: sqlx::Error) -> AppError {
    if error
        .as_database_error()
        .is_some_and(sqlx::error::DatabaseError::is_unique_violation)
    {
        return AppError::bad_request("clientId already belongs to another OAuth app");
    }
    AppError::Db(error)
}

#[cfg(test)]
mod tests {
    use futures::future::join_all;
    use sqlx::PgPool;

    use super::*;

    async fn install_oauth_apps(pool: &PgPool) -> anyhow::Result<Uuid> {
        sqlx::query(
            "CREATE TABLE oauth_apps (
                id uuid PRIMARY KEY,
                name text NOT NULL,
                client_id text NOT NULL,
                client_secret text NOT NULL,
                redirect_uri text NOT NULL,
                active boolean NOT NULL DEFAULT true,
                client_kind text NOT NULL DEFAULT 'default',
                app_id text,
                app_version text,
                user_agent text,
                last_used_at timestamptz,
                created_at timestamp NOT NULL DEFAULT now(),
                updated_at timestamp NOT NULL DEFAULT now()
            )",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            r"CREATE UNIQUE INDEX oauth_apps_client_identity_uq
             ON oauth_apps (
                 btrim(
                     client_id,
                     U&'\0009\000A\000B\000C\000D\0020\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000'
                 )
             )",
        )
        .execute(pool)
        .await?;
        let app_id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO oauth_apps (id, name, client_id, client_secret, redirect_uri)
             VALUES ($1, 'primary', 'client', 'secret', 'http://localhost/callback')",
        )
        .bind(app_id)
        .execute(pool)
        .await?;
        Ok(app_id)
    }

    #[sqlx::test(migrations = false)]
    async fn picker_waits_for_the_only_locked_app(pool: PgPool) -> anyhow::Result<()> {
        let app_id = install_oauth_apps(&pool).await?;
        let mut lock = pool.begin().await?;
        sqlx::query("SELECT id FROM oauth_apps WHERE id = $1 FOR UPDATE")
            .bind(app_id)
            .fetch_one(&mut *lock)
            .await?;
        let service = OAuthAppsService::new(pool);
        let picker = tokio::spawn(async move { service.pick_lru_from(&[app_id]).await });

        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        lock.commit().await?;
        let picked = picker.await??;

        assert_eq!(picked.id, app_id);
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn concurrent_picks_share_the_only_active_app(pool: PgPool) -> anyhow::Result<()> {
        let app_id = install_oauth_apps(&pool).await?;
        let service = OAuthAppsService::new(pool);
        let picks = (0..32).map(|_| {
            let service = Arc::clone(&service);
            async move { service.pick_lru_from(&[app_id]).await }
        });

        let picked = join_all(picks)
            .await
            .into_iter()
            .collect::<AppResult<Vec<_>>>()?;

        assert!(picked.iter().all(|app| app.id == app_id));
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn picker_returns_service_unavailable_without_an_active_app(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let app_id = install_oauth_apps(&pool).await?;
        sqlx::query("UPDATE oauth_apps SET active = false WHERE id = $1")
            .bind(app_id)
            .execute(&pool)
            .await?;
        let service = OAuthAppsService::new(pool);

        let error = service.pick_lru_from(&[app_id]).await.unwrap_err();

        assert_eq!(error.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn client_identity_cannot_change_in_place(pool: PgPool) -> anyhow::Result<()> {
        let app_id = install_oauth_apps(&pool).await?;
        let service = OAuthAppsService::new(pool.clone());

        let error = service
            .update(
                &app_id.to_string(),
                &UpdateOAuthAppDto {
                    client_id: Some("another-client".to_owned()),
                    ..Default::default()
                },
            )
            .await
            .expect_err("client identity must be immutable");
        let client_id: String =
            sqlx::query_scalar("SELECT client_id FROM oauth_apps WHERE id = $1")
                .bind(app_id)
                .fetch_one(&pool)
                .await?;

        assert!(matches!(error, AppError::BadRequest(_)));
        assert_eq!(client_id, "client");
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn normalized_client_identity_is_unique(pool: PgPool) -> anyhow::Result<()> {
        install_oauth_apps(&pool).await?;
        let service = OAuthAppsService::new(pool);

        let error = service
            .create(&web_dto(" client ", "https://example.com/callback"))
            .await
            .expect_err("normalized duplicate must be rejected");

        assert!(matches!(error, AppError::BadRequest(_)));
        Ok(())
    }

    fn web_dto(client_id: &str, redirect_uri: &str) -> CreateOAuthAppDto {
        CreateOAuthAppDto {
            name: "app".to_owned(),
            client_id: client_id.to_owned(),
            client_secret: "secret".to_owned(),
            redirect_uri: redirect_uri.to_owned(),
            active: None,
            client_kind: ClientKind::Default,
            app_id: None,
            app_version: None,
            user_agent: None,
        }
    }

    fn mobile_dto(client_id: &str, redirect_uri: &str) -> CreateOAuthAppDto {
        CreateOAuthAppDto {
            client_kind: ClientKind::Mobile,
            app_id: Some("app.id".to_owned()),
            app_version: Some("1.0".to_owned()),
            user_agent: Some("agent".to_owned()),
            ..web_dto(client_id, redirect_uri)
        }
    }

    async fn insert_mobile_app(pool: &PgPool) -> anyhow::Result<Uuid> {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO oauth_apps
                 (id, name, client_id, client_secret, redirect_uri, client_kind, app_id, app_version, user_agent)
             VALUES ($1, 'mobile', 'mobile-client', 'secret', 'sc://auth', 'mobile', 'app.id', '1.0', 'agent')",
        )
        .bind(id)
        .execute(pool)
        .await?;
        Ok(id)
    }

    #[sqlx::test(migrations = false)]
    async fn pickers_skip_mobile_apps(pool: PgPool) -> anyhow::Result<()> {
        let web = install_oauth_apps(&pool).await?;
        let mobile = insert_mobile_app(&pool).await?;
        let service = OAuthAppsService::new(pool.clone());

        let picked = service.pick_lru_from(&[web, mobile]).await?;
        let only_mobile = service.pick_lru_from(&[mobile]).await.unwrap_err();
        let held = sqlx::query_scalar::<_, Option<chrono::DateTime<chrono::Utc>>>(
            "SELECT last_used_at FROM oauth_apps WHERE id = $1",
        )
        .bind(mobile)
        .fetch_one(&pool)
        .await?;

        assert_eq!(picked.id, web);
        assert_eq!(picked.client_kind, "default");
        assert_eq!(
            only_mobile.status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        );
        assert!(held.is_none());
        assert_eq!(service.count_active().await?, 1);
        assert_eq!(service.find_all().await?.len(), 2);
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn mobile_redirect_is_allowed_only_for_mobile_apps(pool: PgPool) -> anyhow::Result<()> {
        install_oauth_apps(&pool).await?;
        let service = OAuthAppsService::new(pool);

        let web = service.create(&web_dto("web-2", "sc://auth")).await;
        let mobile = service.create(&mobile_dto("mobile-2", "sc://auth")).await?;
        let other_scheme = service.create(&mobile_dto("mobile-3", "sc://other")).await;
        let mobile_web = service
            .create(&mobile_dto("mobile-4", "https://example.com/callback"))
            .await?;

        assert!(matches!(web, Err(AppError::BadRequest(_))));
        assert!(matches!(other_scheme, Err(AppError::BadRequest(_))));
        assert_eq!(mobile.client_kind, "mobile");
        assert_eq!(mobile.app_version.as_deref(), Some("1.0"));
        assert_eq!(mobile_web.redirect_uri, "https://example.com/callback");
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn mobile_app_needs_its_profile_fields(pool: PgPool) -> anyhow::Result<()> {
        install_oauth_apps(&pool).await?;
        let service = OAuthAppsService::new(pool);
        let mut dto = mobile_dto("mobile-5", "sc://auth");
        dto.user_agent = Some("  ".to_owned());

        let error = service
            .create(&dto)
            .await
            .expect_err("user agent is required");

        assert!(matches!(error, AppError::BadRequest(_)));
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn update_keeps_the_redirect_rule_and_kind(pool: PgPool) -> anyhow::Result<()> {
        let web = install_oauth_apps(&pool).await?;
        let service = OAuthAppsService::new(pool.clone());
        let mobile = insert_mobile_app(&pool).await?;

        let web_redirect = service
            .update(
                &web.to_string(),
                &UpdateOAuthAppDto {
                    redirect_uri: Some("sc://auth".to_owned()),
                    ..Default::default()
                },
            )
            .await;
        let kind_change = service
            .update(
                &web.to_string(),
                &UpdateOAuthAppDto {
                    client_kind: Some(ClientKind::Mobile),
                    ..Default::default()
                },
            )
            .await;
        let mobile_update = service
            .update(
                &mobile.to_string(),
                &UpdateOAuthAppDto {
                    redirect_uri: Some("sc://auth".to_owned()),
                    app_version: Some("2.0".to_owned()),
                    ..Default::default()
                },
            )
            .await?;

        assert!(matches!(web_redirect, Err(AppError::BadRequest(_))));
        assert!(matches!(kind_change, Err(AppError::BadRequest(_))));
        assert_eq!(mobile_update.app_version.as_deref(), Some("2.0"));
        Ok(())
    }
}
