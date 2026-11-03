use std::net::SocketAddr;
use std::time::Duration;

use http_body_util::{BodyExt, Empty};
use hyper::body::Bytes;
use hyper::client::conn::http1::{self, SendRequest};
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;

const ROUTES: [&str; 6] = [
    "/fast/1", "/slow", "/spawned", "/nested", "/fast/2", "/broken",
];

#[derive(Default)]
pub struct Sent {
    pub requests: usize,
    pub nested: usize,
    pub debug: Vec<String>,
}

impl Sent {
    pub fn add(&mut self, other: Sent) {
        self.requests += other.requests;
        self.nested += other.nested;
        self.debug.extend(other.debug);
    }
}

async fn open(addr: SocketAddr) -> SendRequest<Empty<Bytes>> {
    let stream = TcpStream::connect(addr).await.expect("connect");
    let (sender, connection) = http1::handshake(TokioIo::new(stream))
        .await
        .expect("handshake");
    tokio::spawn(connection);
    sender
}

async fn get(sender: &mut SendRequest<Empty<Bytes>>, request: hyper::Request<Empty<Bytes>>) -> u16 {
    sender.ready().await.expect("ready");
    let response = sender.send_request(request).await.expect("response");
    let status = response.status().as_u16();
    let _ = response.into_body().collect().await;
    status
}

pub async fn connection(
    addr: SocketAddr,
    stamp: u64,
    first: u64,
    requests: u64,
    pause: Duration,
) -> Sent {
    let mut sender = open(addr).await;
    let mut sent = Sent::default();
    for n in first..first + requests {
        let route = ROUTES[n as usize % ROUTES.len()];
        let mut request = hyper::Request::get(route).header("host", "probe");
        let trace = format!("{stamp:016x}{n:016x}");
        let span = format!("{:016x}", n + 1);
        if n % 64 == 0 && route != "/broken" {
            request = request.header("traceparent", format!("00-{trace}-{span}-01"));
            sent.debug.push(trace);
        } else if n % 2 == 0 {
            request = request.header("traceparent", format!("00-{trace}-{span}-00"));
        }
        get(&mut sender, request.body(Empty::new()).expect("request")).await;
        sent.requests += 1;
        if route == "/nested" {
            sent.nested += 1;
        }
        tokio::time::sleep(pause).await;
    }
    sent
}

pub async fn kept(jaeger: SocketAddr, traces: &[String]) -> usize {
    let mut found = 0;
    for trace in traces {
        let mut sender = open(jaeger).await;
        let request = hyper::Request::get(format!("/jaeger/api/traces/{trace}"))
            .header("host", "jaeger")
            .body(Empty::new())
            .expect("request");
        if get(&mut sender, request).await == 200 {
            found += 1;
        }
    }
    found
}
