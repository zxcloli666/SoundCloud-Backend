use std::time::Duration;

use bytes::{Bytes, BytesMut};
use serde::{Deserialize, Serialize};
use tokio::time::Instant;

use crate::b64;
use crate::transport::{Failure, Outer};

pub const END: &str = "x-pro-end";
pub const PIECE_WAIT: Duration = Duration::from_secs(12);
pub const CLOSE_WAIT: Duration = Duration::from_secs(5);

#[derive(Debug, Serialize, Deserialize)]
pub struct Open {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Opened {
    pub id: String,
    pub status: u16,
    pub headers: Vec<(String, String)>,
}

pub async fn exchange(
    client: &reqwest::Client,
    node: &str,
    target: &str,
    envelope: Vec<u8>,
    timeout: Duration,
    piece: usize,
) -> Result<Outer, Failure> {
    let deadline = Instant::now() + timeout;
    let open = Open {
        method: "POST".to_owned(),
        url: target.to_owned(),
        headers: vec![(
            "content-type".to_owned(),
            "application/octet-stream".to_owned(),
        )],
        body: Some(b64::padded(&envelope)),
    };
    let response = client
        .post(format!("{node}/open"))
        .timeout(timeout)
        .json(&open)
        .send()
        .await
        .map_err(|error| Failure::from_reqwest(&error))?;
    if !response.status().is_success() {
        return Err(Failure::Pro(response.status().as_u16()));
    }
    let opened: Opened = response
        .json()
        .await
        .map_err(|error| Failure::Link(error.to_string()))?;
    let answer = format!("{node}/take/{}", opened.id);
    let body = take(client, &answer, piece, deadline).await;
    client.delete(&answer).timeout(CLOSE_WAIT).send().await.ok();
    Ok(Outer {
        status: opened.status,
        retry_after: retry_after(&opened.headers),
        body: body?,
    })
}

async fn take(
    client: &reqwest::Client,
    answer: &str,
    piece: usize,
    deadline: Instant,
) -> Result<Bytes, Failure> {
    let mut body = BytesMut::new();
    loop {
        if Instant::now() >= deadline {
            return Err(Failure::Timeout);
        }
        let response = client
            .get(format!("{answer}?from={}&max={piece}", body.len()))
            .timeout(PIECE_WAIT)
            .send()
            .await
            .map_err(|error| Failure::from_reqwest(&error))?;
        if !response.status().is_success() {
            return Err(Failure::Pro(response.status().as_u16()));
        }
        let end = response
            .headers()
            .get(END)
            .is_some_and(|value| value.as_bytes() == b"1");
        let bytes = response
            .bytes()
            .await
            .map_err(|error| Failure::from_reqwest(&error))?;
        body.extend_from_slice(&bytes);
        if end {
            return Ok(body.freeze());
        }
    }
}

fn retry_after(headers: &[(String, String)]) -> Option<Duration> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("retry-after"))
        .and_then(|(_, value)| value.trim().parse().ok())
        .map(Duration::from_secs)
}

#[cfg(test)]
#[path = "tests/pro_tests.rs"]
mod tests;
