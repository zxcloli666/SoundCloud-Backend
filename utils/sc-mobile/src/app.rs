use std::fmt;

use redact::Secret;
use serde::Deserialize;

#[derive(Clone, Deserialize)]
pub struct MobileApp {
    pub client_id: String,
    pub client_secret: Secret<String>,
    pub app_id: String,
    pub app_version: String,
    pub user_agent: String,
}

impl fmt::Debug for MobileApp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("MobileApp").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_profile_prints_none_of_its_values() {
        let app = MobileApp {
            client_id: "dummy-client".to_owned(),
            client_secret: Secret::new("dummy-secret".to_owned()),
            app_id: "dummy-app".to_owned(),
            app_version: "dummy-version".to_owned(),
            user_agent: "dummy-agent".to_owned(),
        };

        let printed = format!("{app:?} {app:#?}");

        for value in [
            "dummy-client",
            "dummy-secret",
            "dummy-app",
            "dummy-version",
            "dummy-agent",
        ] {
            assert!(!printed.contains(value), "{printed}");
        }
    }

    #[test]
    fn a_profile_reads_from_the_owner_file_shape() {
        let app: Result<MobileApp, _> = serde_json::from_str(
            r#"{"client_id":"a","client_secret":"b","app_id":"c","app_version":"d","user_agent":"e"}"#,
        );

        assert!(app.is_ok_and(|app| app.client_secret.expose() == "b"));
    }
}
