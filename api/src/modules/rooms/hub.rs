use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures::StreamExt;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::bus::nats::NatsService;

pub const ROOMS_SUBJECT: &str = "rooms.changed";
const CHANNEL_CAPACITY: usize = 1024;
const PUBLISH_BUDGET: Duration = Duration::from_millis(250);

pub struct RoomHub {
    tx: broadcast::Sender<String>,
    nats: Option<Arc<NatsService>>,
}

impl RoomHub {
    pub fn new(nats: Option<Arc<NatsService>>) -> Arc<Self> {
        let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        Arc::new(Self { tx, nats })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }

    pub async fn announce(&self, code: &str) {
        let _ = self.tx.send(code.to_owned());
        let Some(nats) = &self.nats else { return };
        let publish = nats.publish_core(ROOMS_SUBJECT, Bytes::from(code.to_owned()));
        match tokio::time::timeout(PUBLISH_BUDGET, publish).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => warn!(%error, "room change was not fanned out"),
            Err(_) => warn!("room change fan-out timed out"),
        }
    }

    pub fn spawn_bridge(self: &Arc<Self>, shutdown: CancellationToken) {
        let Some(nats) = self.nats.clone() else {
            return;
        };
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let mut subscriber = match nats.subscribe(ROOMS_SUBJECT).await {
                Ok(subscriber) => subscriber,
                Err(error) => {
                    warn!(%error, "room fan-out is local only");
                    return;
                }
            };
            loop {
                tokio::select! {
                    () = shutdown.cancelled() => break,
                    message = subscriber.next() => {
                        let Some(message) = message else { break };
                        if let Ok(code) = std::str::from_utf8(&message.payload) {
                            let _ = tx.send(code.to_owned());
                        }
                    }
                }
            }
        });
    }
}
