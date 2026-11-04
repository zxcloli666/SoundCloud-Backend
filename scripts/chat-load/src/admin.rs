use std::path::Path;
use std::time::Duration;

use anyhow::{Context, bail};
use serde_json::{Value, json};
use tokio::time::{Instant, sleep};

pub const TOKEN_HEADER: &str = "x-chat-admin-token";
pub const ADMIN_HEADER: &str = "x-chat-admin-user";
pub const ADMIN_NAME: &str = "chat-load";
pub const PURGE_PAUSE: Duration = Duration::from_secs(2);

pub struct Admin {
    client: reqwest::Client,
    base: String,
    token: String,
}

impl Admin {
    pub fn new(base: &str, token_file: &Path) -> anyhow::Result<Self> {
        let token = std::fs::read_to_string(token_file)
            .with_context(|| format!("token file {} is not readable", token_file.display()))?
            .trim()
            .to_owned();
        if token.is_empty() {
            bail!("token file {} is empty", token_file.display());
        }
        Ok(Self {
            client: reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(120))
                .build()?,
            base: base.trim_end_matches('/').to_owned(),
            token,
        })
    }

    async fn post(&self, path: &str, body: Option<Value>) -> anyhow::Result<Value> {
        let mut request = self
            .client
            .post(format!("{}{path}", self.base))
            .header(TOKEN_HEADER, &self.token)
            .header(ADMIN_HEADER, ADMIN_NAME);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            bail!(
                "{path} answered {status}: {}",
                text.chars().take(300).collect::<String>()
            );
        }
        serde_json::from_str(&text).with_context(|| format!("{path} answered no json"))
    }

    pub async fn seed(
        &self,
        users: u32,
        devices_per_user: u32,
        groups: u32,
    ) -> anyhow::Result<Value> {
        let body = json!({"users": users, "devices_per_user": devices_per_user, "groups": groups});
        self.post("/chat-admin/loadtest/seed", Some(body)).await
    }

    pub async fn purge(&self, wait: Duration) -> anyhow::Result<Value> {
        let deadline = Instant::now() + wait;
        loop {
            let purged = self.post("/chat-admin/loadtest/purge", None).await?;
            println!("purge: {purged}");
            if purged["pending"].as_i64() == Some(0) {
                return Ok(purged);
            }
            if Instant::now() >= deadline {
                bail!("purge still has {} chats pending", purged["pending"]);
            }
            sleep(PURGE_PAUSE).await;
        }
    }
}
