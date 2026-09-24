use chrono::{DateTime, Utc};
use serde::Deserialize;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(FromRow)]
pub struct ClaimedApp {
    pub id: Uuid,
    pub client_id: String,
    pub client_secret: String,
    pub refresh_token: Option<String>,
    pub refresh_attempts: Option<i32>,
    pub lease_id: Uuid,
}

impl ClaimedApp {
    pub fn refresh_attempts(&self) -> i32 {
        self.refresh_attempts.unwrap_or_default()
    }
}

#[derive(Debug, FromRow)]
pub struct Reservation {
    pub reservation_id: Option<Uuid>,
    pub retry_at: DateTime<Utc>,
}

pub struct Token {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub scope: Option<String>,
    pub expires_in: i64,
}

#[derive(Deserialize)]
pub struct TokenResponse {
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub scope: Option<String>,
    pub expires_in: Option<i64>,
}

impl TryFrom<TokenResponse> for Token {
    type Error = ();

    fn try_from(response: TokenResponse) -> Result<Self, Self::Error> {
        let Some(access_token) = non_empty(response.access_token) else {
            return Err(());
        };
        let Some(expires_in) = response.expires_in.filter(|value| *value > 0) else {
            return Err(());
        };
        Ok(Self {
            access_token,
            refresh_token: non_empty(response.refresh_token),
            scope: non_empty(response.scope),
            expires_in,
        })
    }
}

impl Token {
    pub fn from_refresh(response: TokenResponse) -> Result<Self, ()> {
        let token = Self::try_from(response)?;
        if token.refresh_token.is_some() {
            Ok(token)
        } else {
            Err(())
        }
    }

    pub fn from_client_credentials(response: TokenResponse) -> Result<Self, ()> {
        Self::try_from(response)
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incomplete_token_response_is_rejected() {
        let response = TokenResponse {
            access_token: Some("access".to_owned()),
            refresh_token: None,
            scope: None,
            expires_in: Some(0),
        };

        assert!(Token::try_from(response).is_err());
    }

    #[test]
    fn empty_rotated_refresh_token_rejects_refresh_grant_success() {
        let response = TokenResponse {
            access_token: Some("access".to_owned()),
            refresh_token: Some(String::new()),
            scope: None,
            expires_in: Some(3600),
        };

        assert!(Token::from_refresh(response).is_err());
    }

    #[test]
    fn client_credentials_preserves_its_refresh_token() {
        let response = TokenResponse {
            access_token: Some("access".to_owned()),
            refresh_token: Some("refresh".to_owned()),
            scope: None,
            expires_in: Some(3600),
        };

        assert!(
            Token::from_client_credentials(response)
                .is_ok_and(|token| token.refresh_token.as_deref() == Some("refresh"))
        );
    }

    #[test]
    fn whitespace_credentials_are_incomplete() {
        let response = TokenResponse {
            access_token: Some("  ".to_owned()),
            refresh_token: Some("refresh".to_owned()),
            scope: None,
            expires_in: Some(3600),
        };

        assert!(Token::from_refresh(response).is_err());
    }
}
