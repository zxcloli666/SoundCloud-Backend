use std::path::PathBuf;
use std::sync::Arc;

use async_nats::ConnectOptions;
use async_nats::jetstream;
use serde_json::json;
use tracing::{error, info};

use crate::spool::Spool;

const TRACK_UPLOADED: &str = "storage.track_uploaded";
const TRACK_REJECTED: &str = "storage.track_rejected";

#[derive(Clone)]
pub struct BusClient {
    spool: Option<Arc<Spool>>,
}

impl BusClient {
    pub async fn connect(url: &str, spool_dir: &str) -> Self {
        if url.is_empty() {
            return Self { spool: None };
        }
        let spool = Spool::open(PathBuf::from(spool_dir))
            .await
            .unwrap_or_else(|e| panic!("event spool {spool_dir} is not writable: {e}"));
        let spool = Arc::new(spool);
        let (host_url, user, pass) = split_creds(url);
        let mut opts = ConnectOptions::new()
            .max_reconnects(None)
            .retry_on_initial_connect();
        if let (Some(u), Some(p)) = (user, pass) {
            opts = opts.user_and_password(u, p);
        }
        if let Some(tls) = tls_common::ClientIdentity::from_env("NATS") {
            opts = opts
                .require_tls(true)
                .add_root_certificates(tls.ca)
                .add_client_certificate(tls.cert, tls.key);
        }
        match opts.connect(&host_url).await {
            Ok(client) => {
                info!("NATS client started for {host_url}, event spool {spool_dir}");
                tokio::spawn(spool.clone().run(jetstream::new(client)));
            }
            Err(e) => {
                error!("NATS {host_url} is not usable: {e}; storage events stay in {spool_dir}")
            }
        }
        Self { spool: Some(spool) }
    }

    pub fn enabled(&self) -> bool {
        self.spool.is_some()
    }

    pub async fn publish_track_rejected(
        &self,
        sc_track_id: String,
        reason: &'static str,
        actual_secs: f64,
        expected_duration_ms: Option<i64>,
    ) {
        self.spool_event(
            TRACK_REJECTED,
            json!({
                "sc_track_id": sc_track_id,
                "reason": reason,
                "actual_secs": actual_secs,
                "expected_duration_ms": expected_duration_ms,
            }),
        )
        .await;
    }

    pub async fn publish_track_uploaded(
        &self,
        sc_track_id: String,
        storage_url: String,
        quality: &'static str,
    ) {
        self.spool_event(
            TRACK_UPLOADED,
            json!({
                "sc_track_id": sc_track_id,
                "storage_url": storage_url,
                "quality": quality,
            }),
        )
        .await;
    }

    async fn spool_event(&self, subject: &str, payload: serde_json::Value) {
        let Some(spool) = &self.spool else {
            return;
        };
        let track = payload["sc_track_id"].clone();
        if let Err(e) = spool.push(subject, payload).await {
            error!("[bus] {subject} for track {track} could not be spooled: {e}");
        }
    }
}

fn split_creds(url: &str) -> (String, Option<String>, Option<String>) {
    let (scheme, rest) = url.split_once("://").unwrap_or(("nats", url));
    let Some((creds, host)) = rest.rsplit_once('@') else {
        return (url.to_string(), None, None);
    };
    let host_url = format!("{scheme}://{host}");
    match creds.split_once(':') {
        Some((u, p)) => (host_url, Some(u.to_string()), Some(p.to_string())),
        None => (host_url, Some(creds.to_string()), None),
    }
}

/// `soundcloud_tracks_{id}` → `Some("{id}")`. Returns None for non-numeric tails
/// (we only emit events for canonical SC track URNs).
pub fn sc_track_id_from_filename(filename: &str) -> Option<String> {
    let track = crate::backend::track_of_filename(filename)?;
    (track.storage_name() == filename).then(|| track.sc_id())
}

#[cfg(test)]
mod tests {
    use super::sc_track_id_from_filename;

    #[test]
    fn only_a_canonical_track_object_names_a_track_event() {
        assert_eq!(
            sc_track_id_from_filename("soundcloud_tracks_42").as_deref(),
            Some("42")
        );
        for filename in [
            "42",
            "soundcloud_users_42",
            "remix_42",
            "soundcloud_tracks_042",
            "soundcloud_tracks_",
        ] {
            assert_eq!(sc_track_id_from_filename(filename), None, "{filename}");
        }
    }
}
