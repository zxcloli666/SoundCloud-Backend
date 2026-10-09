use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll};

use futures_util::future::{ready, Either, Ready};
use http::Request;
use tower::{Layer, Service};
use wreq::header::{HeaderMap, OrigHeaderMap};
use wreq::Body;

#[derive(Clone)]
pub(crate) struct CallSiteFirst {
    emulated: Arc<OrigHeaderMap>,
}

#[derive(Clone)]
pub(crate) struct Ordered<S> {
    inner: S,
    emulated: Arc<OrigHeaderMap>,
}

impl CallSiteFirst {
    pub(crate) fn new(emulation: &wreq::Emulation) -> Self {
        let mut emulated = emulation.orig_headers.clone();
        for name in emulation.headers.keys() {
            if !emulation
                .orig_headers
                .iter()
                .any(|(known, _)| known == name)
            {
                emulated.insert(name.clone());
            }
        }
        Self {
            emulated: Arc::new(emulated),
        }
    }
}

impl<S> Layer<S> for CallSiteFirst {
    type Service = Ordered<S>;

    fn layer(&self, inner: S) -> Ordered<S> {
        Ordered {
            inner,
            emulated: self.emulated.clone(),
        }
    }
}

impl<S> Service<Request<Body>> for Ordered<S>
where
    S: Service<Request<Body>>,
    S::Error: From<wreq::Error>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Either<S::Future, Ready<Result<S::Response, S::Error>>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), S::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<Body>) -> Self::Future {
        let carrier = match carrier() {
            Some(carrier) if !request.headers().is_empty() => carrier,
            _ => return Either::Left(self.inner.call(request)),
        };
        let order = call_site_first(request.headers(), &self.emulated);
        match wreq::RequestBuilder::from_parts(carrier.clone(), request.into())
            .orig_headers(order)
            .build()
        {
            Ok(ordered) => Either::Left(self.inner.call(ordered.into())),
            Err(error) => Either::Right(ready(Err(error.into()))),
        }
    }
}

pub(crate) fn call_site_first(call_site: &HeaderMap, emulated: &OrigHeaderMap) -> OrigHeaderMap {
    let mut order = OrigHeaderMap::with_capacity(call_site.keys_len() + emulated.len());
    for name in call_site.keys() {
        match emulated.iter().find(|(known, _)| *known == name) {
            Some((_, case)) => order.insert(case.clone()),
            None => order.insert(name.clone()),
        };
    }
    for (name, case) in emulated {
        if !call_site.contains_key(name) {
            order.insert(case.clone());
        }
    }
    order
}

fn carrier() -> Option<&'static wreq::Client> {
    static CARRIER: OnceLock<Option<wreq::Client>> = OnceLock::new();
    CARRIER
        .get_or_init(|| wreq::Client::builder().no_proxy().build().ok())
        .as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wreq::header::HeaderValue;

    fn names(order: &OrigHeaderMap) -> Vec<&str> {
        order.iter().map(|(name, _)| name.as_str()).collect()
    }

    #[test]
    fn call_site_headers_lead_and_take_their_emulated_names_along() {
        let mut emulated = OrigHeaderMap::new();
        for name in [
            "sec-ch-ua",
            "user-agent",
            "accept",
            "accept-encoding",
            "priority",
        ] {
            emulated.insert(name);
        }
        let mut call_site = HeaderMap::new();
        call_site.insert("authorization", HeaderValue::from_static("OAuth 2-x"));
        call_site.insert("accept", HeaderValue::from_static("application/json"));
        call_site.insert("x-target", HeaderValue::from_static("aGk="));

        let order = call_site_first(&call_site, &emulated);

        assert_eq!(
            names(&order),
            [
                "authorization",
                "accept",
                "x-target",
                "sec-ch-ua",
                "user-agent",
                "accept-encoding",
                "priority",
            ]
        );
    }

    #[test]
    fn without_an_emulated_order_only_the_call_site_is_ordered() {
        let mut call_site = HeaderMap::new();
        call_site.insert("range", HeaderValue::from_static("bytes=0-"));

        let order = call_site_first(&call_site, &OrigHeaderMap::new());

        assert_eq!(names(&order), ["range"]);
    }
}
