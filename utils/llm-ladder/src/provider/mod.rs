mod anthropic;
mod body;
mod fault;
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
    fn start(&self, request: Arc<Request>, time_left: Duration) -> StepStream;
}

pub(crate) struct Provider {
    pub id: String,
    pub price: Price,
    pub total_timeout: Duration,
    pub hedged: bool,
    pub streams: bool,
    backend: Box<dyn Backend>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Price {
    input: u64,
    cache_read: u64,
    cache_write: u64,
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
            price: Price::of(settings),
            total_timeout: Duration::from_millis(settings.total_timeout_ms),
            hedged: settings.hedge.unwrap_or(true),
            streams: settings.kind != Kind::Worker && settings.stream.unwrap_or(true),
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
            hedged: true,
            streams: true,
            backend,
        }
    }

    pub fn start(&self, request: Arc<Request>, time_left: Duration) -> StepStream {
        self.backend
            .start(request, time_left.min(self.total_timeout))
    }
}

impl Price {
    #[cfg(test)]
    pub fn per_million_tokens(input: u64, output: u64) -> Self {
        Self {
            input,
            cache_read: input,
            cache_write: input,
            output,
        }
    }

    pub fn is_set(&self) -> bool {
        self.parts().into_iter().any(|price| price > 0)
    }

    pub fn cost(&self, usage: &Usage) -> u64 {
        let counts = [
            usage.input_tokens,
            usage.cache_read_tokens,
            usage.cache_write_tokens,
            usage.output_tokens,
        ];
        let micro: u128 = counts
            .into_iter()
            .zip(self.parts())
            .map(|(tokens, price)| u128::from(tokens) * u128::from(price))
            .sum();
        u64::try_from(micro.div_ceil(TOKENS_PER_PRICE_UNIT)).unwrap_or(u64::MAX)
    }

    fn of(settings: &ProviderSettings) -> Self {
        let input = micro_usd(settings.input_usd_per_mtok);
        Self {
            input,
            cache_read: settings.cache_read_usd_per_mtok.map_or(input, micro_usd),
            cache_write: settings.cache_write_usd_per_mtok.map_or(input, micro_usd),
            output: micro_usd(settings.output_usd_per_mtok),
        }
    }

    fn parts(&self) -> [u64; 4] {
        [self.input, self.cache_read, self.cache_write, self.output]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;

    fn priced(more: &str) -> Price {
        let config = format!(
            "[providers.paid]\nkind = \"anthropic_compatible\"\n\
             endpoint = \"https://llm.example/v1/messages\"\nmodel = \"example-model\"\n\
             api_key = \"env:EXAMPLE_KEY\"\nmax_output_tokens = 600\ntotal_timeout_ms = 4000\n\
             input_usd_per_mtok = 2.0\noutput_usd_per_mtok = 8.0\n{more}\n"
        );
        let (settings, _): (Settings, toml::Table) = Settings::parse(&config).unwrap();
        Price::of(&settings.providers["paid"])
    }

    #[test]
    fn cost_is_tokens_times_price_rounded_up_to_a_micro_dollar() {
        let price = Price::per_million_tokens(2_000_000, 8_000_000);
        let usage = Usage {
            input_tokens: 1_000,
            output_tokens: 100,
            ..Usage::default()
        };
        assert_eq!(price.cost(&usage), 2_800);
        assert_eq!(
            price.cost(&Usage {
                input_tokens: 1,
                ..Usage::default()
            }),
            2
        );
        assert_eq!(price.cost(&Usage::default()), 0);
        assert!(price.is_set() && !Price::default().is_set());
        assert_eq!(Price::default().cost(&usage), 0);
    }

    #[test]
    fn cached_input_is_charged_at_its_own_prices() {
        let price = priced("cache_read_usd_per_mtok = 0.2\ncache_write_usd_per_mtok = 2.5");
        let usage = Usage {
            input_tokens: 40,
            cache_read_tokens: 3_000,
            cache_write_tokens: 200,
            output_tokens: 70,
        };
        assert_eq!(price.cost(&usage), 80 + 600 + 500 + 560);
        let read_only = Usage {
            cache_read_tokens: 3,
            ..Usage::default()
        };
        assert_eq!(price.cost(&read_only), 1);
    }

    #[test]
    fn a_missing_cache_price_is_the_input_price() {
        let usage = Usage {
            input_tokens: 40,
            cache_read_tokens: 3_000,
            cache_write_tokens: 200,
            output_tokens: 70,
        };
        assert_eq!(priced("").cost(&usage), 80 + 6_000 + 400 + 560);
        assert_eq!(
            priced("").cost(&usage),
            priced("").cost(&Usage {
                input_tokens: usage.prompt_tokens(),
                output_tokens: 70,
                ..Usage::default()
            })
        );
        assert_eq!(
            priced("cache_read_usd_per_mtok = 0.2").cost(&usage),
            80 + 600 + 400 + 560
        );
        assert_eq!(
            priced("cache_write_usd_per_mtok = 2.5").cost(&usage),
            80 + 6_000 + 500 + 560
        );
        assert_eq!(
            priced("cache_read_usd_per_mtok = 0.0").cost(&usage),
            80 + 400 + 560,
            "a zero price is a price, not a missing one"
        );
    }
}
