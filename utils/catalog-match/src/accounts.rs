use serde_json::Value;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;
use wt::MatchScAccount;

const VERIFIED_IDENTITY_INDEX: &str = "artist_sc_accounts_verified_identity_uq";

const RESERVED_SC_PATHS: [&str; 11] = [
    "discover", "search", "you", "stream", "feed", "messages", "settings", "tags", "stations",
    "embed", "pages",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountRole {
    Main,
    Demo,
    Alt,
}

impl AccountRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Demo => "demo",
            Self::Alt => "alt",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "main" => Some(Self::Main),
            "demo" => Some(Self::Demo),
            "alt" => Some(Self::Alt),
            _ => None,
        }
    }
}

pub async fn upsert_account(
    pool: &PgPool,
    artist_id: Uuid,
    sc_user_id: &str,
    role: AccountRole,
    source: &str,
    verified: bool,
) -> Result<(), sqlx::Error> {
    wt::retry(|| async {
        let mut transaction = pool.begin().await?;
        upsert_account_in(
            &mut transaction,
            artist_id,
            sc_user_id,
            role,
            source,
            verified,
        )
        .await?;
        transaction.commit().await
    })
    .await
}

pub async fn upsert_account_in(
    connection: &mut PgConnection,
    artist_id: Uuid,
    sc_user_id: &str,
    role: AccountRole,
    source: &str,
    verified: bool,
) -> Result<(), sqlx::Error> {
    if sc_user_id.is_empty() {
        return Ok(());
    }
    let upsert = MatchScAccount::Upsert {
        artist_id,
        sc_user_id: sc_user_id.to_owned(),
        role: role.as_str().to_owned(),
        source: source.to_owned(),
        verified,
        at: wt::now(),
    };
    wt::call(connection, &upsert).await?;
    Ok(())
}

pub async fn delete_account(
    pool: &PgPool,
    artist_id: Uuid,
    sc_user_id: &str,
) -> Result<bool, sqlx::Error> {
    wt::retry(|| async {
        let mut transaction = pool.begin().await?;
        let deleted = delete_account_in(&mut transaction, artist_id, sc_user_id).await?;
        transaction.commit().await?;
        Ok(deleted)
    })
    .await
}

pub async fn delete_account_in(
    connection: &mut PgConnection,
    artist_id: Uuid,
    sc_user_id: &str,
) -> Result<bool, sqlx::Error> {
    let delete = MatchScAccount::Delete {
        artist_id,
        sc_user_id: sc_user_id.to_owned(),
    };
    wt::call(connection, &delete).await
}

pub fn claims_another_artist(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(database)
        if database.constraint() == Some(VERIFIED_IDENTITY_INDEX))
}

pub fn extract_sc_user_id(value: &Value) -> Option<String> {
    if let Some(kind) = value.get("kind").and_then(Value::as_str)
        && kind != "user"
    {
        return None;
    }
    if let Some(id) = value
        .get("urn")
        .and_then(Value::as_str)
        .and_then(|urn| urn.rsplit(':').next())
        .filter(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Some(id.to_owned());
    }
    value
        .get("id")
        .and_then(Value::as_i64)
        .map(|id| id.to_string())
}

pub fn is_soundcloud_url(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(&url.to_lowercase()) else {
        return false;
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let host = host.strip_prefix("www.").unwrap_or(host);
    if host != "soundcloud.com" && host != "m.soundcloud.com" {
        return false;
    }
    let first_segment = parsed
        .path()
        .trim_start_matches('/')
        .split('/')
        .next()
        .unwrap_or("");
    !first_segment.is_empty() && !RESERVED_SC_PATHS.contains(&first_segment)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_artist_profile_url_is_an_identity_link() {
        assert!(is_soundcloud_url("https://soundcloud.com/ultimathule"));
        assert!(is_soundcloud_url(
            "https://m.soundcloud.com/ultimathule/sets/ep"
        ));
    }

    #[test]
    fn navigation_urls_are_not_identity_links() {
        assert!(!is_soundcloud_url("https://soundcloud.com/discover"));
        assert!(!is_soundcloud_url("https://soundcloud.com/"));
        assert!(!is_soundcloud_url("https://example.com/ultimathule"));
    }

    #[test]
    fn a_resolved_user_yields_its_numeric_id() {
        let user = serde_json::json!({ "kind": "user", "urn": "soundcloud:users:1737058172" });

        assert_eq!(extract_sc_user_id(&user).as_deref(), Some("1737058172"));
    }

    #[test]
    fn a_resolved_track_is_not_an_account() {
        let track = serde_json::json!({ "kind": "track", "urn": "soundcloud:tracks:42" });

        assert_eq!(extract_sc_user_id(&track), None);
    }
}
