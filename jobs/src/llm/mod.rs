mod metrics;
#[cfg(test)]
pub(crate) mod test_support;
mod worker;

use std::sync::Arc;

use anyhow::Context;
use llm_ladder::{Hosting, Ladder, Ladders, WorkerTransport};

use crate::bus::Bus;
use crate::config::AiConfig;

pub use metrics::{AiMethod, AiOutcome, buckets, record_answer, start_at_zero};

use self::metrics::LadderMetrics;
use self::worker::BusWorker;

pub fn refine_ladder(config: &AiConfig, bus: &Bus) -> anyhow::Result<Option<Arc<Ladder>>> {
    let Some(refine) = &config.refine else {
        return Ok(None);
    };
    let hosting = Hosting {
        jetstream: Some(bus.jetstream().clone()),
        worker: Some(Arc::new(BusWorker::new(bus.clone())) as Arc<dyn WorkerTransport>),
        observer: Arc::new(LadderMetrics),
    };
    let ladders = Ladders::build(&refine.settings, hosting)
        .context("the LLM_CONFIG ladders could not be built")?;
    ladders
        .get(&refine.ladder)
        .map(Some)
        .with_context(|| format!("LLM_CONFIG has no ladder {}", refine.ladder))
}
