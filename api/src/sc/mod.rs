pub mod egress;
pub mod errors;
pub mod health;
pub mod read;

#[cfg(test)]
#[path = "read_metrics_tests.rs"]
mod read_metrics_tests;

#[cfg(test)]
#[path = "read_tests.rs"]
mod read_tests;

pub use egress::{EGRESS_APP, PgEgressHealth};
pub use errors::{
    RETRY_AFTER_MAX_SECONDS, classify, is_app_credentials_error, is_ban_error, is_invalid_grant,
    is_rate_limited, is_upstream_failure, retry_after_seconds,
};
pub use health::{FetchStrategy, hedge, race, within_budget};
pub use read::ScReadService;
pub use sc_transport::{OAuthCredentials, ScClient, ScMe, ScTokenResponse};

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    fn sources() -> Vec<(PathBuf, String)> {
        let mut out = Vec::new();
        let mut stack = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|ext| ext == "rs")
                    && let Ok(body) = fs::read_to_string(&path)
                {
                    out.push((path, body));
                }
            }
        }
        out
    }

    #[test]
    fn the_raw_apiv2_proxy_stays_inside_the_read_facade() {
        let leaked: Vec<String> = sources()
            .into_iter()
            .filter(|(path, body)| {
                body.contains("Apiv2Proxy")
                    && !path.ends_with("sc/read.rs")
                    && !path.ends_with("sc/mod.rs")
            })
            .map(|(path, _)| path.display().to_string())
            .collect();

        assert!(
            leaked.is_empty(),
            "Apiv2Proxy must stay inside ScReadService so every public read keeps the relay chain \
             and the breaker; it leaked into: {leaked:?}"
        );
    }
}
