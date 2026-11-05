use std::sync::Arc;

use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

const CHANNEL_CAPACITY: usize = 1024;

pub struct RoomHub {
    tx: broadcast::Sender<String>,
    closing: CancellationToken,
}

impl RoomHub {
    pub fn new() -> Arc<Self> {
        let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        Arc::new(Self {
            tx,
            closing: CancellationToken::new(),
        })
    }

    pub fn close(&self) {
        self.closing.cancel();
    }

    pub fn is_closed(&self) -> bool {
        self.closing.is_cancelled()
    }

    pub async fn closed(&self) {
        self.closing.cancelled().await;
    }

    pub fn listen(&self) -> RoomChanges {
        RoomChanges {
            local: self.tx.subscribe(),
        }
    }

    pub fn announce(&self, code: &str) {
        let _ = self.tx.send(code.to_owned());
    }
}

pub struct RoomChanges {
    local: broadcast::Receiver<String>,
}

impl RoomChanges {
    pub async fn changed(&mut self, code: &str) {
        loop {
            match self.local.recv().await {
                Ok(changed) if changed == code => return,
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(_)) => return,
                Err(broadcast::error::RecvError::Closed) => {
                    return std::future::pending().await;
                }
            }
        }
    }
}
