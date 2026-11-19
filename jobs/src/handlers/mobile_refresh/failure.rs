use std::time::Duration;

use backend_contracts::auth::RefreshOutcome;
use sc_mobile::MobileError;

const TRANSIENT_RETRY_MIN_SECONDS: u64 = 30;
const TRANSIENT_RETRY_MAX_SECONDS: u64 = 15 * 60;
const RATE_LIMIT_RETRY_SECONDS: i64 = 5 * 60;
const APP_CREDENTIALS_RETRY_SECONDS: i64 = 30 * 60;
const INVALID_GRANT: &str = "invalid_grant";
const INVALID_CLIENT: &str = "invalid_client";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Failure {
    Relink,
    AppCredentials,
    RateLimited,
    Transient,
}

impl Failure {
    pub(super) fn of(error: &MobileError) -> Self {
        let MobileError::Refused { status, error, .. } = error else {
            return Self::Transient;
        };
        match (status, error.as_deref()) {
            (_, Some(INVALID_CLIENT)) => Self::AppCredentials,
            (_, Some(INVALID_GRANT)) | (401, _) => Self::Relink,
            (429, _) => Self::RateLimited,
            _ => Self::Transient,
        }
    }

    pub(super) fn kind(self) -> &'static str {
        match self {
            Self::Relink => "relink_required",
            Self::AppCredentials => "app_credentials",
            Self::RateLimited => "rate_limited",
            Self::Transient => "temporarily_unavailable",
        }
    }

    pub(super) fn outcome(self) -> RefreshOutcome {
        match self {
            Self::Relink => RefreshOutcome::ReauthorizationRequired,
            Self::RateLimited => RefreshOutcome::RateLimited,
            Self::AppCredentials | Self::Transient => RefreshOutcome::RetryLater,
        }
    }

    pub(super) fn retry_seconds(self, failure_count: i32) -> i64 {
        match self {
            Self::RateLimited => RATE_LIMIT_RETRY_SECONDS,
            Self::AppCredentials => APP_CREDENTIALS_RETRY_SECONDS,
            Self::Relink | Self::Transient => backoff(failure_count),
        }
    }

    pub(super) fn app_wide_seconds(self) -> Option<i64> {
        match self {
            Self::RateLimited => Some(RATE_LIMIT_RETRY_SECONDS),
            Self::AppCredentials => Some(APP_CREDENTIALS_RETRY_SECONDS),
            Self::Relink | Self::Transient => None,
        }
    }
}

fn backoff(failure_count: i32) -> i64 {
    let exponent = u32::try_from(failure_count.max(0)).map_or(0, |value| value.min(5));
    let seconds = Duration::from_secs(TRANSIENT_RETRY_MIN_SECONDS)
        .saturating_mul(2_u32.saturating_pow(exponent))
        .min(Duration::from_secs(TRANSIENT_RETRY_MAX_SECONDS))
        .as_secs();
    i64::try_from(seconds).unwrap_or(RATE_LIMIT_RETRY_SECONDS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(status: u16, code: Option<&str>) -> MobileError {
        MobileError::Refused {
            status,
            error: code.map(str::to_owned),
            description: None,
        }
    }

    #[test]
    fn a_rejected_refresh_token_asks_for_a_relink() {
        assert_eq!(
            Failure::of(&refused(400, Some("invalid_grant"))),
            Failure::Relink
        );
        assert_eq!(Failure::of(&refused(401, None)), Failure::Relink);
    }

    #[test]
    fn a_rejected_client_is_the_app_and_not_the_user() {
        assert_eq!(
            Failure::of(&refused(401, Some("invalid_client"))),
            Failure::AppCredentials
        );
    }

    #[test]
    fn a_captcha_or_a_server_error_is_retried_later_without_a_relink() {
        assert_eq!(Failure::of(&MobileError::Captcha), Failure::Transient);
        assert_eq!(Failure::of(&refused(503, None)), Failure::Transient);
        assert_eq!(Failure::of(&refused(429, None)), Failure::RateLimited);
    }

    #[test]
    fn backoff_grows_and_stays_bounded() {
        assert!(backoff(1) > backoff(0));
        assert_eq!(backoff(i32::MAX), 15 * 60);
    }
}
