use std::sync::Arc;
use std::time::Duration;

use catalog_sources::{ExternalFetcher, GeniusCfg, GeniusService, LyricsSources, MbClient};

use crate::ClientBuildError;
use crate::config::{EnrichConfig, LyricsConfig};

#[derive(Clone)]
pub struct ExternalSources {
    pub musicbrainz: Arc<MbClient>,
    pub genius: Arc<GeniusService>,
    pub lyrics: Arc<LyricsSources>,
    pub lyrics_on_demand: Arc<LyricsSources>,
}

impl ExternalSources {
    pub fn build(
        relay: Option<Arc<call_relay::Client>>,
        enrich: &EnrichConfig,
        lyrics: &LyricsConfig,
    ) -> Result<Self, ClientBuildError> {
        let fetcher = ExternalFetcher::admitting(
            http_client(lyrics.concurrency)?,
            enrich.proxy_url.clone(),
            relay,
            lyrics.concurrency,
        );
        let background = fetcher.background();
        let genius = GeniusService::new(
            background.clone(),
            GeniusCfg {
                access_token: enrich.genius_access_token.expose().clone(),
                max_concurrent_scrapes: enrich.genius_max_concurrent_scrapes,
            },
        );
        let lyrics = LyricsSources::new(
            background.clone(),
            genius.clone(),
            lyrics.musixmatch_base.clone(),
        );
        Ok(Self {
            musicbrainz: MbClient::new(background, enrich.musicbrainz_rate_limit_ms),
            genius,
            lyrics_on_demand: lyrics.with_fetcher(fetcher),
            lyrics,
        })
    }
}

fn http_client(proxy_requests: usize) -> Result<wreq::Client, ClientBuildError> {
    sc_fingerprint::builder(None)
        .tcp_keepalive(Duration::from_secs(60))
        .pool_max_idle_per_host(proxy_requests.max(20))
        .pool_idle_timeout(Duration::from_secs(90))
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(ClientBuildError::from)
}
