use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures::StreamExt;
use tokio::sync::broadcast;
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

    pub async fn listen(&self) -> RoomChanges {
        let local = self.tx.subscribe();
        let remote = match &self.nats {
            Some(nats) => match nats.subscribe(ROOMS_SUBJECT).await {
                Ok(subscriber) => Some(subscriber),
                Err(error) => {
                    warn!(%error, "room fan-out is local only");
                    None
                }
            },
            None => None,
        };
        RoomChanges { local, remote }
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
}

pub struct RoomChanges {
    local: broadcast::Receiver<String>,
    remote: Option<async_nats::Subscriber>,
}

async fn remote_change(remote: &mut Option<async_nats::Subscriber>, code: &str) {
    loop {
        let Some(subscriber) = remote.as_mut() else {
            return std::future::pending().await;
        };
        match subscriber.next().await {
            Some(message) if message.payload.as_ref() == code.as_bytes() => return,
            Some(_) => {}
            None => *remote = None,
        }
    }
}

impl RoomChanges {
    pub async fn changed(&mut self, code: &str) {
        let Self { local, remote } = self;
        loop {
            tokio::select! {
                () = remote_change(remote, code) => return,
                received = local.recv() => match received {
                    Ok(changed) if changed == code => return,
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(_)) => return,
                    Err(broadcast::error::RecvError::Closed) => {
                        return remote_change(remote, code).await;
                    }
                },
            }
        }
    }
}
