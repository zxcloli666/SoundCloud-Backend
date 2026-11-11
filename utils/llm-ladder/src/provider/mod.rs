mod anthropic;
mod body;
mod http;
mod openai;
mod sse;
pub mod worker;

use std::sync::Arc;
use std::time::Duration;

use futures::stream::BoxStream;

pub(crate) use http::client;

use anthropic::Messages;
use http::Http;
use openai::ChatCompletions;

use crate::config::{Kind, ProviderSettings, micro_usd};
use crate::observer::Failure;
use crate::request::{Request, Usage};
use worker::WorkerTransport;

const TOKENS_PER_PRICE_UNIT: u128 = 1_000_000;

pub(crate) type StepStream = BoxStream<'static, Result<StepEvent, Failure>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StepEvent {
    Text(String),
    Usage(Usage),
}

pub(crate) trait Backend: Send + Sync {
    fn start(&self, request: Arc<Request>) -> StepStream;
}

pub(crate) struct Provider {
    pub id: String,
    pub price: Price,
    pub total_timeout: Duration,
    backend: Box<dyn Backend>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Price {
    input: u64,
    output: u64,
}

impl Provider {
    pub fn build(
        id: &str,
        settings: &ProviderSettings,
        http: &wreq::Client,
        worker: Option<&Arc<dyn WorkerTransport>>,
    ) -> Option<Self> {
        let backend: Box<dyn Backend> = match settings.kind {
            Kind::AnthropicCompatible => Box::new(Http::<Messages>::new(id, settings, http)?),
            Kind::OpenaiCompatible => Box::new(Http::<ChatCompletions>::new(id, settings, http)?),
            Kind::Worker => Box::new(worker::Worker::new(id, settings, worker?.clone())),
        };
        Some(Self {
            id: id.to_owned(),
            price: Price {
                input: micro_usd(settings.input_usd_per_mtok),
                output: micro_usd(settings.output_usd_per_mtok),
            },
            total_timeout: Duration::from_millis(settings.total_timeout_ms),
            backend,
        })
    }

    #[cfg(test)]
    pub fn with_backend(
        id: &str,
        price: Price,
        total_timeout: Duration,
        backend: Box<dyn Backend>,
    ) -> Self {
        Self {
            id: id.to_owned(),
            price,
            total_timeout,
            backend,
        }
    }

    pub fn start(&self, request: Arc<Request>) -> StepStream {
        self.backend.start(request)
    }
}

impl Price {
    #[cfg(test)]
    pub fn per_million_tokens(input: u64, output: u64) -> Self {
        Self { input, output }
    }

    pub fn is_set(&self) -> bool {
        self.input > 0 || self.output > 0
    }

    pub fn cost(&self, usage: &Usage) -> u64 {
        let micro = u128::from(usage.input_tokens) * u128::from(self.input)
            + u128::from(usage.output_tokens) * u128::from(self.output);
        u64::try_from(micro.div_ceil(TOKENS_PER_PRICE_UNIT)).unwrap_or(u64::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_is_tokens_times_price_rounded_up_to_a_micro_dollar() {
        let price = Price::per_million_tokens(2_000_000, 8_000_000);
        let usage = Usage {
            input_tokens: 1_000,
            output_tokens: 100,
        };
        assert_eq!(price.cost(&usage), 2_800);
        assert_eq!(
            price.cost(&Usage {
                input_tokens: 1,
                output_tokens: 0
            }),
            2
        );
        assert_eq!(price.cost(&Usage::default()), 0);
        assert!(price.is_set() && !Price::default().is_set());
        assert_eq!(Price::default().cost(&usage), 0);
    }
}
