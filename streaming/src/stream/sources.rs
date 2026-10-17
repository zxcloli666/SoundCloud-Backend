use std::sync::Arc;

use axum::body::Body;
use axum::http::StatusCode;
use axum::response::Response;
use bytes::{Bytes, BytesMut};
use futures::stream::BoxStream;
use futures::{Sink, SinkExt, StreamExt};
use tracing::{info, warn};

use super::free_hq::FreeSlot;
use super::quality;
use super::restricted::RestrictedSource;
use crate::AppState;
use crate::error::AppError;

pub(super) type Fetched = (Bytes, &'static str, &'static str);
type Decrypted = BoxStream<'static, Result<Bytes, decrypt::Error>>;
type Keep = Box<dyn FnOnce(Bytes) + Send>;

const CHUNKS_IN_FLIGHT: usize = 2;

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
    slot: Option<Arc<FreeSlot>>,
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

    let keep = upload.takes(quality).then(|| {
        let storage = state.storage.clone();
        let urn = track_urn.to_string();
        Box::new(move |whole: Bytes| storage.upload_in_background_with_quality(urn, whole, quality))
            as Keep
    });
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Keeping {
    Nothing,
    WhileHeard,
    Whole,
}

fn decrypt_to_the_end(stream: Decrypted, keep: Option<Keep>, slot: Option<Arc<FreeSlot>>) -> Body {
    let keeping = match (&keep, &slot) {
        (None, _) => Keeping::Nothing,
        (Some(_), None) => Keeping::WhileHeard,
        (Some(_), Some(_)) => Keeping::Whole,
    };
    if slot.is_some() {
        let (listener, heard) = futures::channel::mpsc::unbounded();
        tokio::spawn(finish(pump(stream, listener, keeping), keep, slot));
        return Body::from_stream(heard);
    }
    let (listener, heard) = futures::channel::mpsc::channel(CHUNKS_IN_FLIGHT);
    tokio::spawn(finish(pump(stream, listener, keeping), keep, slot));
    Body::from_stream(heard)
}

async fn finish(
    pumped: impl Future<Output = Option<Bytes>>,
    keep: Option<Keep>,
    slot: Option<Arc<FreeSlot>>,
) {
    let whole = pumped.await;
    drop(slot);
    if let (Some(keep), Some(whole)) = (keep, whole)
        && whole.len() > 8192
    {
        keep(whole);
    }
}

async fn pump<L>(mut stream: Decrypted, mut listener: L, keeping: Keeping) -> Option<Bytes>
where
    L: Sink<Result<Bytes, decrypt::Error>> + Unpin,
{
    let mut kept = BytesMut::new();
    let mut heard = true;
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(e) => {
                if heard {
                    let _ = listener.send(Err(e)).await;
                }
                return None;
            }
        };
        if keeping != Keeping::Nothing {
            kept.extend_from_slice(&chunk);
        }
        if heard && listener.send(Ok(chunk)).await.is_err() {
            if keeping != Keeping::Whole {
                return None;
            }
            heard = false;
        }
    }
    drop(listener);
    (keeping != Keeping::Nothing).then(|| kept.freeze())
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
    use crate::stream::free_hq::{FreeHq, Gate};
    use futures::channel::mpsc::Sender;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    const URN: &str = "soundcloud:tracks:1";

    fn free_slot(free: &FreeHq) -> Arc<FreeSlot> {
        let Gate::Open(Some(slot)) = free.gate(URN, "listener", false) else {
            panic!("the slot was free");
        };
        slot
    }

    fn stored_copy() -> (Keep, tokio::sync::oneshot::Receiver<usize>) {
        let (stored_tx, stored) = tokio::sync::oneshot::channel();
        let keep: Keep = Box::new(move |whole: Bytes| {
            let _ = stored_tx.send(whole.len());
        });
        (keep, stored)
    }

    fn chunks(count: usize) -> Decrypted {
        futures::stream::iter((0..count).map(|_| Ok(Bytes::from_static(b"abcd")))).boxed()
    }

    fn counted(count: usize) -> (Decrypted, Arc<AtomicUsize>) {
        let pulled = Arc::new(AtomicUsize::new(0));
        let counter = pulled.clone();
        let stream = chunks(count)
            .inspect(move |_| {
                counter.fetch_add(1, Ordering::SeqCst);
            })
            .boxed();
        (stream, pulled)
    }

    fn gone_listener() -> Sender<Result<Bytes, decrypt::Error>> {
        let (listener, gone) = futures::channel::mpsc::channel(CHUNKS_IN_FLIGHT);
        drop(gone);
        listener
    }

    #[tokio::test]
    async fn a_listener_that_leaves_early_still_leaves_a_whole_copy_for_storage() {
        let whole = pump(chunks(5), gone_listener(), Keeping::Whole).await;

        assert_eq!(
            whole.map(|whole| whole.len()),
            Some(20),
            "the desktop drops the slower of /download and /stream; the decrypt we already \
             paid for must still reach storage"
        );
    }

    #[tokio::test]
    async fn with_nothing_to_keep_the_decrypt_stops_when_the_listener_leaves() {
        let (stream, pulled) = counted(5);

        assert!(
            pump(stream, gone_listener(), Keeping::Nothing)
                .await
                .is_none()
        );
        assert_eq!(pulled.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn without_a_slot_the_copy_is_dropped_with_the_listener() {
        let (stream, pulled) = counted(5);

        assert!(
            pump(stream, gone_listener(), Keeping::WhileHeard)
                .await
                .is_none()
        );
        assert_eq!(
            pulled.load(Ordering::SeqCst),
            1,
            "only a decrypt holding a slot may run on with nobody listening"
        );
    }

    #[tokio::test]
    async fn without_a_slot_a_slow_listener_holds_the_decrypt_back() {
        let (stream, pulled) = counted(50);
        let (listener, heard) = futures::channel::mpsc::channel(CHUNKS_IN_FLIGHT);
        let decrypt = tokio::spawn(pump(stream, listener, Keeping::Nothing));
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }

        assert!(
            pulled.load(Ordering::SeqCst) <= CHUNKS_IN_FLIGHT + 2,
            "a listener that does not read must not make us buffer the whole track"
        );
        drop(heard);
        assert!(decrypt.await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_decrypt_that_breaks_halfway_is_never_kept() {
        let stream = futures::stream::iter([
            Ok(Bytes::from_static(b"abcd")),
            Err(decrypt::Error::Fetch("segment 2".into())),
            Ok(Bytes::from_static(b"efgh")),
        ])
        .boxed();
        let (listener, mut heard) = futures::channel::mpsc::channel(CHUNKS_IN_FLIGHT);

        assert!(pump(stream, listener, Keeping::Whole).await.is_none());
        assert!(heard.next().await.is_some_and(|chunk| chunk.is_ok()));
        assert!(heard.next().await.is_some_and(|chunk| chunk.is_err()));
        assert!(heard.next().await.is_none());
    }

    #[tokio::test]
    async fn the_free_slot_comes_back_only_after_the_detached_decrypt_ends() {
        let free = FreeHq::new(1);
        let (feed, segments) = futures::channel::mpsc::unbounded();
        let (keep, stored) = stored_copy();

        drop(decrypt_to_the_end(
            segments.boxed(),
            Some(keep),
            Some(free_slot(&free)),
        ));
        for _ in 0..3 {
            feed.unbounded_send(Ok(Bytes::from(vec![0u8; 4096])))
                .unwrap();
        }
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
        assert!(
            matches!(free.gate(URN, "another", false), Gate::Busy),
            "the slot bounds decrypts nobody listens to; it must stay taken while one runs"
        );

        drop(feed);
        assert_eq!(stored.await.unwrap(), 3 * 4096);
        assert!(matches!(
            free.gate(URN, "another", false),
            Gate::Open(Some(_))
        ));
    }

    #[tokio::test]
    async fn a_slow_listener_does_not_hold_a_free_slot() {
        let free = FreeHq::new(1);
        let (keep, stored) = stored_copy();
        let segments = futures::stream::iter((0..4).map(|_| Ok(Bytes::from(vec![7u8; 4096]))));

        let body = decrypt_to_the_end(segments.boxed(), Some(keep), Some(free_slot(&free)));
        let stored = tokio::time::timeout(Duration::from_secs(1), stored).await;

        assert_eq!(
            stored.ok().and_then(Result::ok),
            Some(4 * 4096),
            "a client behind a slow relay must not keep a free slot for its whole download"
        );
        assert!(matches!(
            free.gate(URN, "another", false),
            Gate::Open(Some(_))
        ));
        let heard = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        assert_eq!(heard.len(), 4 * 4096);
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
