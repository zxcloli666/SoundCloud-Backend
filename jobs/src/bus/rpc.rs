use std::future::Future;
use std::sync::Arc;

use anyhow::Context;
use futures::StreamExt;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use super::Bus;

impl Bus {
    pub async fn serve<Q, R, F, Fut>(
        &self,
        subject: &'static str,
        queue: &'static str,
        concurrency: usize,
        cancellation: CancellationToken,
        handler: F,
    ) -> anyhow::Result<()>
    where
        Q: DeserializeOwned + Send + 'static,
        R: Serialize + Send + 'static,
        F: Fn(Q) -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = anyhow::Result<R>> + Send + 'static,
    {
        let mut requests = self
            .client
            .queue_subscribe(subject, queue.to_owned())
            .await
            .with_context(|| format!("subscribe to {subject}"))?;
        let slots = Arc::new(Semaphore::new(concurrency.max(1)));
        loop {
            let message = tokio::select! {
                () = cancellation.cancelled() => return Ok(()),
                message = requests.next() => message,
            };
            let Some(message) = message else {
                anyhow::bail!("{subject} subscription closed");
            };
            let Some(reply_to) = message.reply.clone() else {
                continue;
            };
            let permit = slots.clone().acquire_owned().await?;
            let client = self.client.clone();
            let handler = handler.clone();
            tokio::spawn(async move {
                let _permit = permit;
                let answer = match serde_json::from_slice::<Q>(&message.payload) {
                    Ok(request) => handler(request).await,
                    Err(error) => Err(error.into()),
                };
                let body = match answer.and_then(|reply| Ok(serde_json::to_vec(&reply)?)) {
                    Ok(body) => body,
                    Err(error) => {
                        warn!(%subject, error = %format!("{error:#}"), "request failed");
                        Vec::new()
                    }
                };
                if let Err(error) = client.publish(reply_to, body.into()).await {
                    warn!(%subject, %error, "reply was not sent");
                }
            });
        }
    }
}
