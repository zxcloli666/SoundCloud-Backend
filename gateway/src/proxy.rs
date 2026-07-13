use std::convert::Infallible;
use std::net::IpAddr;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{self, HeaderMap, HeaderName, HeaderValue};
use hyper::{Request, Response, StatusCode, Uri, Version};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioIo;
use tracing::debug;

use crate::routes::RouteTable;

pub type ProxyBody = BoxBody<Bytes, Box<dyn std::error::Error + Send + Sync>>;
pub type HttpClient = Client<HttpConnector, Incoming>;

pub struct ProxyState {
    pub client: HttpClient,
    pub routes: Arc<RouteTable>,
    pub https_port: u16,
}

/// Hop-by-hop headers (RFC 7230 §6.1) — stripped on both request and response.
/// `upgrade`/`connection` are conditionally kept for protocol upgrades.
const HOP_NAMES: [&str; 9] = [
    "connection",
    "proxy-connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

pub async fn handle(
    state: Arc<ProxyState>,
    mut req: Request<Incoming>,
    client_ip: IpAddr,
    tls: bool,
) -> Response<ProxyBody> {
    // Host: header (h1) or URI authority (h2).
    let forward_host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .or_else(|| req.uri().authority().map(|a| a.as_str().to_string()));

    let upstream = match forward_host.as_deref().and_then(|h| state.routes.lookup(h)) {
        Some(u) => u.clone(),
        None => return status_resp(StatusCode::NOT_FOUND, "gateway: no route for host\n"),
    };

    let upgrade = is_upgrade(req.headers());

    let path = req.uri().path_and_query().map(|p| p.as_str()).unwrap_or("/");
    let target: Uri = match format!("{}://{}{}", upstream.scheme, upstream.authority, path).parse() {
        Ok(u) => u,
        Err(e) => {
            debug!("bad target uri for {}: {e}", upstream.authority);
            return status_resp(StatusCode::BAD_GATEWAY, "gateway: bad upstream uri\n");
        }
    };

    sanitize(req.headers_mut(), upgrade);
    // Preserve the client's Host verbatim — signed requests (S3 SigV4) hash it.
    if let Some(h) = &forward_host {
        if let Ok(v) = HeaderValue::from_str(h) {
            req.headers_mut().insert(header::HOST, v);
        }
    }
    add_forwarded(req.headers_mut(), client_ip, tls, forward_host.as_deref());

    *req.uri_mut() = target;
    *req.version_mut() = Version::HTTP_11;

    if upgrade {
        return proxy_upgrade(state, req).await;
    }

    match state.client.request(req).await {
        Ok(resp) => {
            let (mut parts, body) = resp.into_parts();
            strip_hop(&mut parts.headers, false);
            Response::from_parts(parts, box_body(body))
        }
        Err(e) => {
            debug!("upstream error: {e}");
            status_resp(StatusCode::BAD_GATEWAY, "gateway: upstream error\n")
        }
    }
}

async fn proxy_upgrade(state: Arc<ProxyState>, mut req: Request<Incoming>) -> Response<ProxyBody> {
    let client_up = hyper::upgrade::on(&mut req);

    let mut resp = match state.client.request(req).await {
        Ok(r) => r,
        Err(e) => {
            debug!("upgrade upstream error: {e}");
            return status_resp(StatusCode::BAD_GATEWAY, "gateway: upstream error\n");
        }
    };

    if resp.status() != StatusCode::SWITCHING_PROTOCOLS {
        // Upstream refused the upgrade — relay its response as-is.
        let (mut parts, body) = resp.into_parts();
        strip_hop(&mut parts.headers, false);
        return Response::from_parts(parts, box_body(body));
    }

    let upstream_up = hyper::upgrade::on(&mut resp);
    tokio::spawn(async move {
        match tokio::try_join!(client_up, upstream_up) {
            Ok((client_io, upstream_io)) => {
                let mut c = TokioIo::new(client_io);
                let mut u = TokioIo::new(upstream_io);
                if let Err(e) = tokio::io::copy_bidirectional(&mut c, &mut u).await {
                    debug!("upgrade tunnel closed: {e}");
                }
            }
            Err(e) => debug!("upgrade handshake failed: {e}"),
        }
    });

    // Relay the 101 back, keeping Connection/Upgrade so hyper upgrades our side too.
    let (parts, body) = resp.into_parts();
    Response::from_parts(parts, box_body(body))
}

pub fn redirect_to_https(req: &Request<Incoming>, https_port: u16) -> Response<ProxyBody> {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(|h| h.split(':').next().unwrap_or(h))
        .unwrap_or("");
    let pq = req.uri().path_and_query().map(|p| p.as_str()).unwrap_or("/");
    let location = if https_port == 443 {
        format!("https://{host}{pq}")
    } else {
        format!("https://{host}:{https_port}{pq}")
    };
    let mut resp = status_resp(StatusCode::MOVED_PERMANENTLY, "");
    if let Ok(v) = HeaderValue::from_str(&location) {
        resp.headers_mut().insert(header::LOCATION, v);
    }
    resp
}

fn is_upgrade(headers: &HeaderMap) -> bool {
    headers.contains_key(header::UPGRADE) && header_has_token(headers, &header::CONNECTION, "upgrade")
}

fn header_has_token(headers: &HeaderMap, name: &HeaderName, token: &str) -> bool {
    headers.get_all(name).iter().any(|v| {
        v.to_str()
            .map(|s| s.split(',').any(|t| t.trim().eq_ignore_ascii_case(token)))
            .unwrap_or(false)
    })
}

/// Removes hop-by-hop request headers plus any header named in `Connection`.
/// When `keep_upgrade`, the `connection`/`upgrade` pair is preserved so the
/// upgrade can be forwarded.
fn sanitize(headers: &mut HeaderMap, keep_upgrade: bool) {
    let listed = connection_tokens(headers);
    strip_hop(headers, keep_upgrade);
    for tok in listed {
        if keep_upgrade && tok == "upgrade" {
            continue;
        }
        if let Ok(name) = HeaderName::from_bytes(tok.as_bytes()) {
            headers.remove(name);
        }
    }
}

fn strip_hop(headers: &mut HeaderMap, keep_upgrade: bool) {
    for name in HOP_NAMES {
        if keep_upgrade && (name == "upgrade" || name == "connection") {
            continue;
        }
        if let Ok(n) = HeaderName::from_bytes(name.as_bytes()) {
            headers.remove(n);
        }
    }
}

fn connection_tokens(headers: &HeaderMap) -> Vec<String> {
    let mut out = Vec::new();
    for v in headers.get_all(header::CONNECTION).iter() {
        if let Ok(s) = v.to_str() {
            for tok in s.split(',') {
                let tok = tok.trim().to_ascii_lowercase();
                if !tok.is_empty() && tok != "close" && tok != "keep-alive" {
                    out.push(tok);
                }
            }
        }
    }
    out
}

fn add_forwarded(headers: &mut HeaderMap, ip: IpAddr, tls: bool, host: Option<&str>) {
    let ip_str = ip.to_string();
    let xff = match headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        Some(prev) => format!("{prev}, {ip_str}"),
        None => ip_str.clone(),
    };
    if let Ok(v) = HeaderValue::from_str(&xff) {
        headers.insert(HeaderName::from_static("x-forwarded-for"), v);
    }
    if let Ok(v) = HeaderValue::from_str(&ip_str) {
        headers.insert(HeaderName::from_static("x-real-ip"), v);
    }
    headers.insert(
        HeaderName::from_static("x-forwarded-proto"),
        HeaderValue::from_static(if tls { "https" } else { "http" }),
    );
    if let Some(h) = host {
        let bare = h.split(':').next().unwrap_or(h);
        if let Ok(v) = HeaderValue::from_str(bare) {
            headers.insert(HeaderName::from_static("x-forwarded-host"), v);
        }
    }
}

fn box_body(body: Incoming) -> ProxyBody {
    body.map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>).boxed()
}

fn status_resp(status: StatusCode, msg: &'static str) -> Response<ProxyBody> {
    let body = Full::new(Bytes::from_static(msg.as_bytes()))
        .map_err(|e: Infallible| match e {})
        .boxed();
    let mut resp = Response::new(body);
    *resp.status_mut() = status;
    resp
}
