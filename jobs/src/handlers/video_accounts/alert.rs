use std::future::Future;
use std::time::Duration;

use redact::Secret;
use wreq::Client;

const TIMEOUT: Duration = Duration::from_secs(10);

pub trait Notify: Send + Sync {
    fn send(&self, text: &str) -> impl Future<Output = Result<(), String>> + Send;
}

pub struct Telegram {
    http: Client,
    webhook: Secret<String>,
}

impl Telegram {
    pub fn new(webhook: Secret<String>) -> Result<Self, wreq::Error> {
        let http = sc_fingerprint::builder(None).timeout(TIMEOUT).build()?;
        Ok(Self { http, webhook })
    }
}

impl Notify for Telegram {
    async fn send(&self, text: &str) -> Result<(), String> {
        if self.webhook.is_empty() {
            return Err("TG_WEBHOOK is not set".to_owned());
        }
        let response = self
            .http
            .post(self.webhook.expose().as_str())
            .form(&[("text", text)])
            .send()
            .await
            .map_err(|_| "the telegram request failed".to_owned())?;
        match response.status().as_u16() {
            200..=299 => Ok(()),
            status => Err(format!("telegram answered {status}")),
        }
    }
}

pub(super) fn dead_text(name: &str, source: &str, error: Option<&str>) -> String {
    let section = match source {
        "youtube" => "YouTube",
        "spotify" => "Spotify",
        _ => "Яндекс Музыка",
    };
    let reason = error.unwrap_or("причина не записана");
    format!(
        "Аккаунт видео {name} ({section}) умер: {reason}. Повторить раздел \"{section}\" в Infra/docs/video-accounts.md."
    )
}
