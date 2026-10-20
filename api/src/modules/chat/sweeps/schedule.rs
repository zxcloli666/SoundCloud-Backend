use std::sync::Arc;
use std::time::Duration;

use tokio::time::Instant;

use crate::modules::chat::ChatState;
use crate::modules::chat::sweeps::{
    devices, expired, gauges, leader, marks, purge, reports, rooms, startup,
};

pub const TICK: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sweep {
    Marks,
    Expired,
    Rooms,
    Purge,
    Devices,
    Reports,
}

impl Sweep {
    pub const ALL: [Sweep; 6] = [
        Sweep::Marks,
        Sweep::Expired,
        Sweep::Rooms,
        Sweep::Purge,
        Sweep::Devices,
        Sweep::Reports,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Sweep::Marks => "marks",
            Sweep::Expired => "expired",
            Sweep::Rooms => "rooms",
            Sweep::Purge => "purge",
            Sweep::Devices => "devices",
            Sweep::Reports => "reports",
        }
    }

    pub fn every(self) -> Duration {
        Duration::from_secs(match self {
            Sweep::Marks => marks::EVERY_SECS,
            Sweep::Expired => expired::EVERY_SECS,
            Sweep::Rooms => rooms::EVERY_SECS,
            Sweep::Purge => purge::EVERY_SECS,
            Sweep::Devices => devices::EVERY_SECS,
            Sweep::Reports => reports::EVERY_SECS,
        })
    }
}

pub struct Sweeper {
    marks: marks::Flusher,
    due: [Instant; Sweep::ALL.len()],
    leading: bool,
}

impl Sweeper {
    pub fn new() -> Self {
        Self {
            marks: marks::Flusher::new(),
            due: [Instant::now(); Sweep::ALL.len()],
            leading: false,
        }
    }

    pub async fn tick(&mut self, chat: &ChatState, leading: bool) {
        let took_lead = leading && !self.leading;
        self.leading = leading;
        if !leading {
            return;
        }
        if took_lead {
            match startup::run_once(chat).await {
                Ok(rebumped) => gauges::done("startup", rebumped),
                Err(error) => {
                    gauges::failed("startup");
                    tracing::warn!(code = error.code(), "chat sweep startup failed");
                }
            }
        }
        for (index, sweep) in Sweep::ALL.into_iter().enumerate() {
            if chat.shutdown.is_cancelled() {
                return;
            }
            if Instant::now() < self.due[index] {
                continue;
            }
            self.due[index] = Instant::now() + sweep.every();
            self.run(chat, sweep).await;
        }
    }

    async fn run(&mut self, chat: &ChatState, sweep: Sweep) {
        let outcome = match sweep {
            Sweep::Marks => self.marks.run_once(chat).await,
            Sweep::Expired => expired::run_once(chat).await,
            Sweep::Rooms => rooms::run_once(chat).await,
            Sweep::Purge => purge::run_once(chat).await,
            Sweep::Devices => devices::run_once(chat).await,
            Sweep::Reports => reports::run_once(chat).await,
        };
        match outcome {
            Ok(items) => gauges::done(sweep.name(), items),
            Err(error) => {
                gauges::failed(sweep.name());
                tracing::warn!(
                    sweep = sweep.name(),
                    code = error.code(),
                    "chat sweep failed"
                );
            }
        }
    }
}

impl Default for Sweeper {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn run(chat: Arc<ChatState>) {
    let mut sweeper = Sweeper::new();
    loop {
        tokio::select! {
            _ = chat.shutdown.cancelled() => return,
            _ = tokio::time::sleep(TICK) => {}
        }
        let leading = match leader::hold(&chat).await {
            Ok(held) => held,
            Err(error) => {
                tracing::warn!(code = error.code(), "chat sweep lease is unknown");
                false
            }
        };
        sweeper.tick(&chat, leading).await;
    }
}
