use axum::body::Body;
use axum::http::StatusCode;
use axum::http::header::{CONTENT_LENGTH, CONTENT_TYPE};
use axum::response::Response;
use bytes::Bytes;
use entity_ref::{EntityKind, EntityRef};
use std::sync::Arc;
use tracing::{debug, info, warn};
use wreq::Client;

use super::breaker::Breaker;
use super::upload_slots::UploadSlots;
use crate::config::Config;
use crate::db::postgres::PgPool;

const UPLOADS_AT_ONCE: usize = 8;
const UPLOAD_BUDGET: std::time::Duration = std::time::Duration::from_secs(300);
pub const PASSTHROUGH_IDLE_BUDGET: std::time::Duration = std::time::Duration::from_secs(15);

pub struct StorageClient {
    client: Client,
    passthrough: Client,
    base_url: String,
    public_url: String,
    upload_url: String,
    auth_token: String,
    pg: PgPool,
    reads: Breaker,
    uploads_failing: Arc<Breaker>,
    uploads: UploadSlots,
}

impl StorageClient {
    pub fn new(client: Client, passthrough: Client, config: &Config, pg: PgPool) -> Self {
        Self {
            client,
            passthrough,
            base_url: config.storage_url.trim_end_matches('/').to_string(),
            public_url: config.storage_public_url.trim_end_matches('/').to_string(),
            upload_url: config.storage_upload_url.trim_end_matches('/').to_string(),
            auth_token: config.storage_token.clone(),
            pg,
            reads: Breaker::default(),
            uploads_failing: Arc::default(),
            uploads: UploadSlots::new(UPLOADS_AT_ONCE),
        }
    }

    pub fn enabled(&self) -> bool {
        !self.base_url.is_empty() && !self.auth_token.is_empty()
    }

    pub fn has_room_for_background_uploads(&self) -> bool {
        self.uploads.half_free()
    }

    pub fn track_filename(track_urn: &str) -> Option<String> {
        is_canonical_track_urn(track_urn)
            .then(|| EntityRef::parse_urn(track_urn))
            .flatten()
            .map(EntityRef::storage_name)
    }

    pub fn track_path(track_urn: &str) -> Option<String> {
        Self::track_filename(track_urn).map(|filename| format!("{filename}.m4a"))
    }

    pub fn internal_url(&self, track_urn: &str) -> Option<String> {
        Self::track_path(track_urn).map(|path| format!("{}/{path}", self.base_url))
    }

    pub fn public_track_url(&self, track_urn: &str) -> Option<String> {
        Self::track_path(track_urn).map(|path| format!("{}/{path}", self.public_url))
    }

    fn is_temporarily_unavailable(&self) -> bool {
        self.reads.is_open()
    }

    pub async fn try_serve(&self, track_urn: &str) -> Option<String> {
        if !self.enabled() || self.is_temporarily_unavailable() {
            return None;
        }

        let verify_url = self.internal_url(track_urn)?;
        let cached = self.pg.find_cached_track(track_urn).await.ok()??;

        match self.verify_url(&verify_url).await {
            VerifyResult::Ok => {
                let _ = self.pg.update_last_accessed(&cached.id).await;
                self.public_track_url(track_urn)
            }
            VerifyResult::Missing => {
                let _ = self.pg.mark_storage_lost(&cached.id, track_urn).await;
                None
            }
            VerifyResult::Unavailable => None,
        }
    }

    pub async fn try_proxy(
        &self,
        track_urn: &str,
        stored_quality: Option<&'static str>,
    ) -> Option<Response> {
        if !self.enabled() || self.is_temporarily_unavailable() {
            return None;
        }

        let internal_url = self.internal_url(track_urn)?;
        let cached = self.pg.find_cached_track(track_urn).await.ok()??;
        let response = match self.passthrough.get(internal_url).send().await {
            Ok(response) if response.status().is_success() => response,
            Ok(response)
                if matches!(response.status(), StatusCode::NOT_FOUND | StatusCode::GONE) =>
            {
                self.mark_available();
                let _ = self.pg.mark_storage_lost(&cached.id, track_urn).await;
                return None;
            }
            Ok(_) | Err(_) => {
                self.mark_unavailable();
                return None;
            }
        };

        self.mark_available();
        let _ = self.pg.update_last_accessed(&cached.id).await;
        let content_type = response.headers().get(CONTENT_TYPE).cloned();
        let content_length = response.headers().get(CONTENT_LENGTH).cloned();
        let mut builder = Response::builder().status(StatusCode::OK);
        if let Some(content_type) = content_type {
            builder = builder.header(CONTENT_TYPE, content_type);
        }
        if let Some(content_length) = content_length {
            builder = builder.header(CONTENT_LENGTH, content_length);
        }
        if let Some(quality) = stored_quality {
            builder = builder.header("x-audio-quality", quality);
        }
        builder
            .body(Body::from_stream(response.bytes_stream()))
            .ok()
    }

    pub fn upload_in_background(&self, track_urn: String, data: Bytes) {
        self.upload_in_background_with_quality(track_urn, data, "sq");
    }

    pub fn upload_in_background_with_quality(
        &self,
        track_urn: String,
        data: Bytes,
        quality: &'static str,
    ) {
        let (Some(filename), Some(verify_target), Some(cdn_path)) = (
            Self::track_filename(&track_urn),
            self.internal_url(&track_urn),
            Self::track_path(&track_urn),
        ) else {
            warn!("[storage] refusing upload for non-canonical urn: {track_urn:?}");
            return;
        };
        if !self.enabled() || self.uploads_failing.is_open() {
            return;
        }
        let Some(slot) = self.uploads.take(&track_urn) else {
            debug!("[storage] {filename} skipped: already uploading or all slots busy");
            return;
        };

        let client = self.client.clone();
        let upload_url = self.upload_url.clone();
        let auth_token = self.auth_token.clone();
        let pg = self.pg.clone();
        let failing = self.uploads_failing.clone();

        tokio::spawn(async move {
            let _slot = slot;
            let id = match pg.insert_cdn_track(&track_urn, &cdn_path, "pending").await {
                Ok(id) => id,
                Err(e) => {
                    warn!("[storage] insert pending failed: {e}");
                    return;
                }
            };

            let expected_ms = lookup_expected_duration_ms(&pg, &track_urn).await;
            match upload_to_storage(
                &client,
                &upload_url,
                &auth_token,
                &filename,
                &data,
                quality,
                expected_ms,
            )
            .await
            {
                Ok(()) => {
                    failing.succeed();
                    let _ = pg.update_cdn_track_status(&id, "ok").await;
                    info!(
                        "[storage] uploaded {} {} ({:.1} MB)",
                        filename,
                        quality,
                        data.len() as f64 / 1024.0 / 1024.0
                    );
                }
                Err(UploadError::Rejected { status, body }) => {
                    failing.succeed();
                    info!("[storage] upload rejected for {filename} ({status}): {body}");
                    let st = settle_failed_status(&client, &verify_target).await;
                    let _ = pg.update_cdn_track_status(&id, st).await;
                }
                Err(e) => {
                    if let Some(failures) = failing.fail() {
                        warn!("[storage] uploads paused after {failures} upload failures");
                    }
                    warn!("[storage] upload failed for {filename}: {e}");
                    let st = settle_failed_status(&client, &verify_target).await;
                    let _ = pg.update_cdn_track_status(&id, st).await;
                }
            }
        });
    }

    async fn verify_url(&self, url: &str) -> VerifyResult {
        if self.is_temporarily_unavailable() {
            return VerifyResult::Unavailable;
        }

        match self
            .client
            .head(url)
            .timeout(std::time::Duration::from_secs(3))
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status().as_u16();
                if (200..300).contains(&status) {
                    self.mark_available();
                    VerifyResult::Ok
                } else if status == 404 || status == 410 {
                    self.mark_available();
                    VerifyResult::Missing
                } else {
                    self.mark_unavailable();
                    VerifyResult::Unavailable
                }
            }
            Err(_) => {
                self.mark_unavailable();
                VerifyResult::Unavailable
            }
        }
    }

    fn mark_available(&self) {
        self.reads.succeed();
    }

    fn mark_unavailable(&self) {
        if let Some(failures) = self.reads.fail() {
            warn!("[storage] breaker opened after {failures} failures");
        }
    }
}

enum VerifyResult {
    Ok,
    Missing,
    Unavailable,
}

#[derive(Debug)]
pub(crate) enum UploadError {
    Rejected { status: u16, body: String },
    Transport(Box<dyn std::error::Error + Send + Sync>),
}

impl std::fmt::Display for UploadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected { status, body } => write!(f, "rejected ({status}): {body}"),
            Self::Transport(e) => write!(f, "{e}"),
        }
    }
}

async fn settle_failed_status(client: &Client, verify_url: &str) -> &'static str {
    let head = client
        .head(verify_url)
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await;
    match head {
        Ok(resp) if resp.status().is_success() => "ok",
        _ => "error",
    }
}

pub(crate) async fn lookup_expected_duration_ms(pg: &PgPool, track_urn: &str) -> Option<i64> {
    let lookup = pg.get_trusted_duration_ms(track_urn);
    match tokio::time::timeout(std::time::Duration::from_secs(3), lookup).await {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            warn!("[storage] expected duration lookup failed for {track_urn}: {e}");
            None
        }
        Err(_) => {
            warn!("[storage] expected duration lookup timed out for {track_urn}");
            None
        }
    }
}

pub(crate) async fn upload_to_storage(
    client: &Client,
    base_url: &str,
    auth_token: &str,
    filename: &str,
    data: &Bytes,
    quality: &str,
    expected_duration_ms: Option<i64>,
) -> Result<(), UploadError> {
    let file_part = wreq::multipart::Part::stream_with_length(data.clone(), data.len() as u64)
        .file_name("audio")
        .mime_str("audio/mpeg")
        .map_err(|e| UploadError::Transport(e.into()))?;

    let mut form = wreq::multipart::Form::new()
        .text("filename", filename.to_string())
        .text("quality", quality.to_string())
        .part("file", file_part);
    if let Some(ms) = expected_duration_ms {
        form = form.text("expected_duration_ms", ms.to_string());
    }

    let resp = client
        .post(format!("{base_url}/upload"))
        .header("Authorization", format!("Bearer {auth_token}"))
        .multipart(form)
        .timeout(UPLOAD_BUDGET)
        .send()
        .await
        .map_err(|e| UploadError::Transport(e.into()))?;

    let status = resp.status();
    if status.is_success() {
        return Ok(());
    }
    if status == wreq::StatusCode::CONFLICT {
        let body: String = resp
            .text()
            .await
            .unwrap_or_default()
            .chars()
            .take(300)
            .collect();
        return Err(UploadError::Rejected {
            status: status.as_u16(),
            body,
        });
    }
    Err(UploadError::Transport(
        format!("storage responded {status}").into(),
    ))
}

pub fn is_canonical_track_urn(track_urn: &str) -> bool {
    entity_ref::is_canonical_urn(EntityKind::Track, track_urn)
}

pub fn canonical_track_urn(input: &str) -> Option<String> {
    entity_ref::track_urn(input)
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::{StorageClient, canonical_track_urn, is_canonical_track_urn, upload_to_storage};

    fn declared_length(head: &str) -> Option<usize> {
        head.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())
                .flatten()
        })
    }

    async fn storage_that_reads_one_upload(listener: TcpListener) -> (String, Vec<u8>) {
        let (mut socket, _) = listener.accept().await.expect("the upload connects");
        let mut request = Vec::new();
        let mut chunk = vec![0_u8; 64 * 1024];
        let (head, body_start) = loop {
            let read = socket
                .read(&mut chunk)
                .await
                .expect("the upload is readable");
            assert!(read > 0, "the upload ended before its headers");
            request.extend_from_slice(&chunk[..read]);
            if let Some(end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                break (
                    String::from_utf8_lossy(&request[..end]).into_owned(),
                    end + 4,
                );
            }
        };
        let length = declared_length(&head).expect("the upload declares its length");
        while request.len() < body_start + length {
            let read = socket
                .read(&mut chunk)
                .await
                .expect("the upload is readable");
            assert!(read > 0, "the upload ended before its declared length");
            request.extend_from_slice(&chunk[..read]);
        }
        socket
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
            .await
            .expect("the answer is written");
        (head, request.split_off(body_start))
    }

    #[tokio::test]
    async fn an_upload_streams_the_whole_track_under_a_declared_length() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a port is free");
        let address = listener.local_addr().expect("the listener has an address");
        let storage = tokio::spawn(storage_that_reads_one_upload(listener));
        let track = Bytes::from(vec![7_u8; 3 << 20]);
        let client = wreq::Client::builder().build().expect("the client builds");

        let outcome = upload_to_storage(
            &client,
            &format!("http://{address}"),
            "token",
            "soundcloud_tracks_1",
            &track,
            "hq",
            Some(180_000),
        )
        .await;

        assert!(outcome.is_ok(), "{outcome:?}");
        let (head, body) = storage.await.expect("the storage stub finished");
        assert_eq!(declared_length(&head), Some(body.len()));
        assert_eq!(
            body.iter().filter(|&&byte| byte == 7).count(),
            track.len(),
            "the multipart body must carry every byte of the track"
        );
    }

    #[test]
    fn canonical_urn_maps_to_canonical_filename() {
        assert!(is_canonical_track_urn("soundcloud:tracks:12345"));
        assert_eq!(
            StorageClient::track_filename("soundcloud:tracks:12345").as_deref(),
            Some("soundcloud_tracks_12345")
        );
        assert_eq!(
            StorageClient::track_path("soundcloud:tracks:12345").as_deref(),
            Some("soundcloud_tracks_12345.m4a")
        );
    }

    #[test]
    fn rejects_bare_and_foreign_urns() {
        for input in [
            "12345",
            "soundcloud:users:12345",
            "soundcloud:tracks:",
            "soundcloud:tracks:abc",
            "soundcloud:tracks:012345",
            "soundcloud_tracks_12345",
            "",
        ] {
            assert!(!is_canonical_track_urn(input), "{input:?}");
            assert_eq!(StorageClient::track_filename(input), None, "{input:?}");
            assert_eq!(StorageClient::track_path(input), None, "{input:?}");
        }
    }

    #[test]
    fn a_bare_id_is_canonicalized_before_it_can_name_an_object() {
        assert_eq!(
            canonical_track_urn("12345").as_deref(),
            Some("soundcloud:tracks:12345")
        );
        assert_eq!(
            canonical_track_urn("soundcloud:tracks:12345").as_deref(),
            Some("soundcloud:tracks:12345")
        );
        assert_eq!(canonical_track_urn("soundcloud:users:12345"), None);
        assert_eq!(canonical_track_urn("012345"), None);
    }
}
