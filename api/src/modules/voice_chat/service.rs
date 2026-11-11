use std::sync::Arc;
use std::time::Instant;

use axum::response::Response;
use llm_ladder::{
    ConfigError, Hosting, Ladder, LadderError, Ladders, Observer, Request, StepReport,
    WorkerTransport,
};

use super::request::Turn;
use super::worker::WorkerRpc;
use super::{prompt, reply};
use crate::bus::nats::NatsService;
use crate::config::{LlmCfg, VoiceChatCfg};

pub struct VoiceChat {
    live: Option<Live>,
}

pub struct Live {
    limits: VoiceChatCfg,
    ladder: Arc<Ladder>,
}

struct LadderMetrics;

impl VoiceChat {
    pub fn start(
        config: &LlmCfg,
        nats: Option<&Arc<NatsService>>,
    ) -> Result<Arc<Self>, ConfigError> {
        let Some(limits) = config.voice_chat.clone() else {
            return Ok(Self::off());
        };
        let hosting = Hosting {
            jetstream: nats.map(|nats| nats.jetstream().clone()),
            worker: nats.map(|nats| WorkerRpc::new(nats.clone()) as Arc<dyn WorkerTransport>),
            observer: Arc::new(LadderMetrics),
        };
        let ladders = Ladders::build(&config.ladders, hosting)?;
        let live = ladders
            .get(&limits.ladder)
            .map(|ladder| Live { limits, ladder });
        Ok(Arc::new(Self { live }))
    }

    pub fn off() -> Arc<Self> {
        Arc::new(Self { live: None })
    }

    pub fn live(&self) -> Option<&Live> {
        self.live.as_ref()
    }
}

impl Live {
    pub fn limits(&self) -> &VoiceChatCfg {
        &self.limits
    }

    pub async fn answer(&self, turn: Turn, user: &str) -> Result<Response, LadderError> {
        let started = Instant::now();
        let request = Request {
            system: prompt::system(&self.limits, &turn),
            messages: prompt::messages(&turn),
            schema: turn.tools.answer_schema(),
            budget_user: Some(user.to_owned()),
            deadline: None,
        };
        let answer = self.ladder.stream(request).await?;
        Ok(reply::respond(answer, turn.tools, started))
    }
}

impl Observer for LadderMetrics {
    fn step(&self, report: &StepReport<'_>) {
        crate::metrics::record_llm_step(report);
    }
}
