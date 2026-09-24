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
}

impl ExternalSources {
    pub fn build(
        relay: Option<Arc<call_relay::Client>>,
        enrich: &EnrichConfig,
        lyrics: &LyricsConfig,
    ) -> Result<Self, ClientBuildError> {
        let fetcher = ExternalFetcher::new(http_client()?, enrich.proxy_url.clone(), relay);
        let genius = GeniusService::new(
            fetcher.clone(),
            GeniusCfg {
                access_token: enrich.genius_access_token.expose().clone(),
                max_concurrent_scrapes: enrich.genius_max_concurrent_scrapes,
            },
        );
        Ok(Self {
            musicbrainz: MbClient::new(fetcher.clone(), enrich.musicbrainz_rate_limit_ms),
            genius: genius.clone(),
            lyrics: LyricsSources::new(fetcher, genius, lyrics.musixmatch_base.clone()),
        })
    }
}

fn http_client() -> Result<wreq::Client, ClientBuildError> {
    sc_fingerprint::builder(None)
        .tcp_keepalive(Duration::from_secs(60))
        .pool_max_idle_per_host(20)
        .pool_idle_timeout(Duration::from_secs(90))
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(ClientBuildError::from)
}
