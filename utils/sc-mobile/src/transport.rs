use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use bytes::{Bytes, BytesMut};
use futures_util::StreamExt;
use url::Url;
use wreq::header::{HeaderMap, LOCATION};
use wreq::{Client, Method, RequestBuilder};

use crate::client::MobileReply;
use crate::error::MobileError;
use crate::plan::UserPlan;

const TARGET_HEADER: &str = "x-target";

pub enum Egress {
    Direct,
    Proxy(Url),
    ProxyThenDirect(Url),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Route<'a> {
    Direct,
    Proxy(&'a Url),
}

pub(crate) struct Prepared {
    pub(crate) method: Method,
    pub(crate) target: Url,
    pub(crate) headers: HeaderMap,
    pub(crate) body: Option<Bytes>,
}

pub(crate) struct Received {
    pub(crate) reply: MobileReply,
    pub(crate) cut: bool,
}

impl Egress {
    pub(crate) fn first(&self) -> Route<'_> {
        match self {
            Self::Direct => Route::Direct,
            Self::Proxy(proxy) | Self::ProxyThenDirect(proxy) => Route::Proxy(proxy),
        }
    }

    pub(crate) fn direct_fallback(&self) -> Option<Route<'_>> {
        match self {
            Self::ProxyThenDirect(_) => Some(Route::Direct),
            Self::Direct | Self::Proxy(_) => None,
        }
    }
}

pub(crate) fn outgoing(http: &Client, prepared: &Prepared, route: Route<'_>) -> RequestBuilder {
    let request = match route {
        Route::Direct => http.request(prepared.method.clone(), prepared.target.as_str()),
        Route::Proxy(proxy) => http
            .request(prepared.method.clone(), proxy.as_str())
            .header(TARGET_HEADER, STANDARD.encode(prepared.target.as_str())),
    }
    .headers(prepared.headers.clone());
    match &prepared.body {
        Some(body) => request.body(body.clone()),
        None => request,
    }
}

pub(crate) async fn send(
    http: &Client,
    prepared: &Prepared,
    route: Route<'_>,
    limit: usize,
) -> Result<Received, MobileError> {
    let response = outgoing(http, prepared, route)
        .send()
        .await
        .map_err(MobileError::transport)?;
    let status = response.status().as_u16();
    let plan = UserPlan::from_headers(response.headers());
    let location = response
        .headers()
        .get(LOCATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| prepared.target.join(value).ok());
    let mut body = BytesMut::new();
    let mut cut = false;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(MobileError::transport)?;
        let room = limit.saturating_sub(body.len());
        if chunk.len() > room {
            body.extend_from_slice(chunk.get(..room).unwrap_or_default());
            cut = true;
            break;
        }
        body.extend_from_slice(&chunk);
    }
    Ok(Received {
        reply: MobileReply {
            status,
            plan,
            location,
            body: body.freeze(),
        },
        cut,
    })
}
