use std::sync::Arc;

use axum::body::Body;
use axum::http::StatusCode;
use axum::response::Response;
use bytes::Bytes;
use futures::StreamExt;
use futures::channel::mpsc::UnboundedSender;
use futures::stream::BoxStream;
use tokio::sync::OwnedSemaphorePermit;
use tracing::{info, warn};

use super::quality;
use super::restricted::RestrictedSource;
use super::storage::StorageClient;
use crate::AppState;
use crate::error::AppError;

pub(super) type Fetched = (Bytes, &'static str, &'static str);

#[derive(Clone, Copy)]
pub(super) struct UploadRule {
    pub cacheable: bool,
    pub kept: Option<&'static str>,
}

impl UploadRule {
    pub(super) fn takes(self, quality: &str) -> bool {
        self.cacheable
            && self
                .kept
                .is_none_or(|kept| quality::rank(quality) > quality::rank(kept))
    }
}

pub(super) async fn try_session_oauth(
    state: &AppState,
    access_token: Option<&str>,
    track_urn: &str,
    secret_token: Option<&str>,
    hq_only: bool,
    premium: bool,
) -> Option<Fetched> {
    let access_token = access_token?;
    let ctx = super::oauth::OauthCtx {
        client: &state.http_client,
        pg: &state.pg,
        proxy_url: &state.config.sc_proxy_url,
        proxy_fallback: may_go_direct(state.config.sc_proxy_fallback, premium, hq_only),
        fallback_session_count: state.config.sc_oauth_fallback_sessions,
    };
    let result =
        super::oauth::try_oauth_stream(&ctx, access_token, track_urn, secret_token, hq_only)
            .await?;
    Some((result.data, result.content_type, result.quality))
}

fn may_go_direct(allowed: bool, premium: bool, hq_only: bool) -> bool {
    allowed && (premium || !hq_only)
}

pub(super) async fn try_cookies(
    state: &AppState,
    track_urn: &str,
    tag: &str,
    hq_only: bool,
    premium: bool,
) -> Option<Fetched> {
    if !premium {
        return None;
    }
    let cookies_client = state.cookies.as_ref()?;
    match cookies_client.get_stream(track_urn, hq_only).await {
        Ok(Some(result)) => Some((result.data, result.content_type, result.quality)),
        Ok(None) => {
            warn!("{tag} {track_urn} cookies returned nothing");
            None
        }
        Err(e) => {
            warn!("{tag} {track_urn} cookies failed: {e}");
            None
        }
    }
}

pub(super) async fn try_anon(
    state: &AppState,
    track_urn: &str,
    tag: &str,
    at_least: &'static str,
) -> Option<Fetched> {
    match state.anon.get_stream(track_urn, at_least).await {
        Ok(Some(result)) => Some((result.data, result.content_type, result.quality)),
        Ok(None) => {
            info!("{tag} {track_urn} anon has nothing at {at_least} or above");
            None
        }
        Err(e) => {
            warn!("{tag} {track_urn} anon failed: {e}");
            None
        }
    }
}

async fn restricted_source(
    state: &AppState,
    track_urn: &str,
    tag: &str,
    at_least: &'static str,
    premium: bool,
) -> Option<RestrictedSource> {
    let uq_first = premium && at_least != quality::SQ;
    match state
        .anon
        .resolve_restricted(track_urn, uq_first, at_least)
        .await
    {
        Ok(Some(v)) => return Some(v),
        Ok(None) => {}
        Err(e) => warn!("{tag} {track_urn} restricted(anon) failed: {e}"),
    }
    if !premium {
        return None;
    }
    let cookies = state.cookies.as_ref()?;
    match cookies
        .resolve_restricted(track_urn, uq_first, at_least)
        .await
    {
        Ok(Some(v)) => Some(v),
        Ok(None) => None,
        Err(e) => {
            warn!("{tag} {track_urn} restricted(cookies) failed: {e}");
            None
        }
    }
}

pub(super) async fn try_relay_track(
    state: &AppState,
    track_urn: &str,
    quality: &'static str,
) -> Option<Fetched> {
    let id = track_urn.rsplit(':').next()?;
    if id.is_empty() || id == track_urn {
        return None;
    }
    let client_id = state.anon.get_client_id().await.unwrap_or_default();
    let (audio, ct) = crate::stream::proxy::get_track_via_relay(
        id,
        quality,
        &client_id,
        state.config.edge_wvd_url.as_deref(),
        state.config.edge_wvd_token.as_deref(),
    )
    .await?;
    Some((
        Bytes::from(audio),
        crate::stream::hls::mime_to_content_type(&ct),
        quality,
    ))
}

pub(super) async fn try_restricted(
    state: &AppState,
    track_urn: &str,
    tag: &str,
    at_least: &'static str,
    premium: bool,
    upload: UploadRule,
    slot: Option<Arc<OwnedSemaphorePermit>>,
) -> Option<(Response, &'static str)> {
    let src = restricted_source(state, track_urn, tag, at_least, premium).await?;
    let quality = src.quality;

    if let (Some(wvd_url), Some(wvd_token)) = (
        state.config.edge_wvd_url.as_deref(),
        state.config.edge_wvd_token.as_deref(),
    ) && let Some(audio) =
        crate::stream::proxy::hls_decrypt_via_relay(&src.manifest, &src.token, wvd_url, wvd_token)
            .await
    {
        let bytes = Bytes::from(audio);
        if upload.takes(quality) && bytes.len() > 8192 {
            state.storage.upload_in_background_with_quality(
                track_urn.to_string(),
                bytes.clone(),
                quality,
            );
        }
        return Some((
            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", src.content_type)
                .header("x-audio-quality", quality)
                .body(Body::from(bytes))
                .unwrap(),
            quality,
        ));
    }

    let engine = state.decryptor.as_ref()?;
    let fetcher: Arc<dyn decrypt::Fetcher> = Arc::new(crate::stream::decrypt_fetch::ProxyFetcher {
        client: state.http_client.clone(),
        proxy_url: state.config.sc_proxy_url.clone(),
    });
    let stream = match engine
        .process_stream(&src.manifest, &src.token, fetcher)
        .await
    {
        Ok(s) => s,
        Err(e) => {
            warn!("{tag} {track_urn} restricted decode failed: {e}");
            return None;
        }
    };

    let keep = upload
        .takes(quality)
        .then(|| (state.storage.clone(), track_urn.to_string(), quality));
    Some((
        Response::builder()
            .status(StatusCode::OK)
            .header("content-type", src.content_type)
            .header("x-audio-quality", quality)
            .body(decrypt_to_the_end(stream, keep, slot))
            .unwrap(),
        quality,
    ))
}

fn decrypt_to_the_end(
    stream: BoxStream<'static, Result<Bytes, decrypt::Error>>,
    keep: Option<(Arc<StorageClient>, String, &'static str)>,
    slot: Option<Arc<OwnedSemaphorePermit>>,
) -> Body {
    let (tx, rx) = futures::channel::mpsc::unbounded();
    tokio::spawn(async move {
        let _slot = slot;
        let whole = pump(stream, tx, keep.is_some()).await;
        if let (Some((storage, urn, quality)), Some(whole)) = (keep, whole)
            && whole.len() > 8192
        {
            storage.upload_in_background_with_quality(urn, Bytes::from(whole), quality);
        }
    });
    Body::from_stream(rx)
}

async fn pump(
    mut stream: BoxStream<'static, Result<Bytes, decrypt::Error>>,
    listener: UnboundedSender<Result<Bytes, decrypt::Error>>,
    keeping: bool,
) -> Option<Vec<u8>> {
    let mut kept = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(e) => {
                let _ = listener.unbounded_send(Err(e));
                return None;
            }
        };
        if keeping {
            kept.push(chunk.clone());
        }
        let listener_left = listener.unbounded_send(Ok(chunk)).is_err();
        if listener_left && !keeping {
            return None;
        }
    }
    drop(listener);
    keeping.then(|| kept.concat())
}

pub(super) fn respond_with_data(
    state: &AppState,
    track_urn: &str,
    (data, content_type, quality): Fetched,
    upload: UploadRule,
) -> Result<Response, AppError> {
    if upload.takes(quality) && data.len() > 8192 {
        state.storage.upload_in_background_with_quality(
            track_urn.to_string(),
            data.clone(),
            quality,
        );
    }

    data_response(data, content_type, quality)
}

pub(super) fn data_response(
    data: Bytes,
    content_type: &'static str,
    quality: &'static str,
) -> Result<Response, AppError> {
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("content-type", content_type)
        .header("content-length", data.len().to_string())
        .header("x-audio-quality", quality)
        .body(Body::from(data))
        .unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn chunks(count: usize) -> BoxStream<'static, Result<Bytes, decrypt::Error>> {
        futures::stream::iter((0..count).map(|_| Ok(Bytes::from_static(b"abcd")))).boxed()
    }

    #[tokio::test]
    async fn a_listener_that_leaves_early_still_leaves_a_whole_copy_for_storage() {
        let (listener, gone) = futures::channel::mpsc::unbounded();
        drop(gone);

        let whole = pump(chunks(5), listener, true).await;

        assert_eq!(
            whole.map(|whole| whole.len()),
            Some(20),
            "the desktop drops the slower of /download and /stream; the decrypt we already \
             paid for must still reach storage"
        );
    }

    #[tokio::test]
    async fn with_nothing_to_keep_the_decrypt_stops_when_the_listener_leaves() {
        let pulled = Arc::new(AtomicUsize::new(0));
        let counter = pulled.clone();
        let stream = chunks(5)
            .inspect(move |_| {
                counter.fetch_add(1, Ordering::SeqCst);
            })
            .boxed();
        let (listener, gone) = futures::channel::mpsc::unbounded();
        drop(gone);

        assert!(pump(stream, listener, false).await.is_none());
        assert_eq!(pulled.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_decrypt_that_breaks_halfway_is_never_kept() {
        let stream = futures::stream::iter([
            Ok(Bytes::from_static(b"abcd")),
            Err(decrypt::Error::Fetch("segment 2".into())),
            Ok(Bytes::from_static(b"efgh")),
        ])
        .boxed();
        let (listener, mut heard) = futures::channel::mpsc::unbounded();

        assert!(pump(stream, listener, true).await.is_none());
        assert!(heard.next().await.is_some_and(|chunk| chunk.is_ok()));
        assert!(heard.next().await.is_some_and(|chunk| chunk.is_err()));
        assert!(heard.next().await.is_none());
    }

    #[test]
    fn a_free_session_reads_apiv1_directly_only_for_sq() {
        assert!(!may_go_direct(true, false, true));
        assert!(may_go_direct(true, false, false));
        assert!(may_go_direct(true, true, true));
        assert!(!may_go_direct(false, true, false));
    }

    #[test]
    fn storage_only_gets_a_copy_better_than_the_one_it_keeps() {
        let empty = UploadRule {
            cacheable: true,
            kept: None,
        };
        assert!(empty.takes(quality::SQ));

        let kept_hq = UploadRule {
            cacheable: true,
            kept: Some(quality::HQ),
        };
        assert!(!kept_hq.takes(quality::SQ));
        assert!(!kept_hq.takes(quality::HQ));
        assert!(kept_hq.takes(quality::UQ));

        let private = UploadRule {
            cacheable: false,
            kept: None,
        };
        assert!(!private.takes(quality::UQ));
    }
}
