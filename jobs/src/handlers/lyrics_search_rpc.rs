use std::sync::Arc;

use backend_contracts::lyrics_search::{
    LYRICS_SEARCH_QUEUE, LYRICS_SEARCH_SUBJECT, LyricsSearchCandidate, LyricsSearchReply,
    LyricsSearchRequest,
};
use catalog_sources::{LrclibRecord, LrclibSearch};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::bus::Bus;

const CONCURRENCY: usize = 8;
const MAX_QUERY_CHARS: usize = 200;

pub struct LyricsSearchRpc {
    lrclib: LrclibSearch,
}

impl LyricsSearchRpc {
    pub fn new(lrclib: LrclibSearch) -> Self {
        Self { lrclib }
    }

    pub async fn serve(
        self: Arc<Self>,
        bus: Bus,
        cancellation: CancellationToken,
    ) -> anyhow::Result<()> {
        bus.serve(
            LYRICS_SEARCH_SUBJECT,
            LYRICS_SEARCH_QUEUE,
            CONCURRENCY,
            cancellation,
            move |request: LyricsSearchRequest| {
                let rpc = self.clone();
                async move { Ok(rpc.reply(request).await) }
            },
        )
        .await
    }

    async fn reply(&self, request: LyricsSearchRequest) -> LyricsSearchReply {
        let query: String = request.query.trim().chars().take(MAX_QUERY_CHARS).collect();
        if query.is_empty() {
            return LyricsSearchReply {
                reachable: true,
                candidates: Vec::new(),
            };
        }
        match self.lrclib.search(&query, request.duration_sec).await {
            Ok(records) => LyricsSearchReply {
                reachable: true,
                candidates: records.into_iter().map(candidate).collect(),
            },
            Err(error) => {
                warn!(%error, "manual lyrics search failed");
                LyricsSearchReply {
                    reachable: false,
                    candidates: Vec::new(),
                }
            }
        }
    }
}

fn candidate(record: LrclibRecord) -> LyricsSearchCandidate {
    LyricsSearchCandidate {
        lrclib_id: record.id,
        artist: record.artist,
        title: record.title,
        album: record.album,
        duration_sec: record.duration_sec,
        synced_lrc: record.synced_lrc,
        plain_text: record.plain_text,
    }
}
