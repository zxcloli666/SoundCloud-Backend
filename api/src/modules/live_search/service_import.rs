use catalog_ingest::Observation;
use serde_json::Value;

use super::{LiveRequest, LiveSearch, LocalPage, Served, now, served_from};
use crate::cache::ListPageResult;
use crate::modules::live_search::merge::{self, SOURCE_LOCAL};
use crate::modules::live_search::meta::{LiveMeta, LivePage, LiveState};
use crate::modules::live_search::query::{ADOPT_CAP, LiveClass, LiveKind, LiveQuery, identity_of};
use crate::modules::tracks::TrackPriority;

impl LiveSearch {
    pub fn match_plan(&self, text: &str, sc_user_id: &str) -> Option<LiveRequest> {
        let limit = LiveClass::Match.window_size(LiveKind::Tracks);
        Some(LiveRequest {
            kind: LiveKind::Tracks,
            class: LiveClass::Match,
            query: LiveQuery::parse(text)?,
            page: 0,
            limit,
            local_limit: limit,
            identity: identity_of(sc_user_id),
        })
    }

    pub async fn hits(&self, request: &LiveRequest) -> (Vec<Value>, LiveMeta) {
        let served = if request.class.allowed_in(self.cfg.mode, self.cfg.db_rescue) {
            match self
                .store
                .window(request.scope(), &request.query.hash)
                .await
            {
                Some(window) if window.is_fresh(now()) => {
                    Served::window(window, Vec::new(), LiveState::Cached)
                }
                previous => served_from(self.live(request).await, previous),
            }
        } else {
            Served::nothing(LiveState::Off, None)
        };
        crate::metrics::record_live_request(
            request.kind.as_str(),
            request.class.as_str(),
            served.meta.state.as_str(),
        );
        let hits = match &served.window {
            Some(window) => self.build(request, &window.ids, 0, &served.items).await,
            None => Vec::new(),
        };
        (hits, served.meta)
    }

    pub(super) async fn import_page(
        &self,
        request: &LiveRequest,
        hits: Vec<Value>,
        after: LocalPage,
        meta: LiveMeta,
    ) -> LivePage {
        if !self.cfg.ranked {
            return top_hit_first(request, hits, after, meta);
        }
        let (mut local, meta) = match after {
            Ok(after) => (after.collection, meta),
            Err(_) => (Vec::new(), meta.local_unavailable()),
        };
        merge::tag_all(&mut local, SOURCE_LOCAL);
        let pick = merge::confident_pick(&request.query.text, hits, local);
        if let Some(pick) = pick.as_ref().filter(|pick| merge::is_live(pick)) {
            self.keep_pick(pick).await;
        }
        LivePage::new(
            ListPageResult {
                collection: pick.into_iter().collect(),
                page: 0,
                page_size: request.limit,
                has_more: false,
            },
            meta,
        )
    }

    pub async fn keep_pick(&self, pick: &Value) {
        let Some(indexing) = self.indexing.get() else {
            return;
        };
        let kept = tokio::time::timeout(
            ADOPT_CAP,
            indexing.ingest_track_from_sc(pick, TrackPriority::Playlist, Observation::UNVERIFIED),
        )
        .await;
        let outcome = if matches!(kept, Ok(Ok(()))) {
            "adopted"
        } else {
            "failed"
        };
        crate::metrics::record_live_adopt("track", outcome);
    }
}

fn top_hit_first(
    request: &LiveRequest,
    hits: Vec<Value>,
    after: LocalPage,
    meta: LiveMeta,
) -> LivePage {
    let (mut collection, local_more, meta) = match after {
        Ok(mut after) => {
            merge::tag_all(&mut after.collection, SOURCE_LOCAL);
            let mut merged = hits;
            merged.extend(after.collection);
            (merged, after.has_more, meta)
        }
        Err(_) => (hits, false, meta.local_unavailable()),
    };
    collection = merge::dedupe_by_urn(collection);
    let limit = usize::try_from(request.limit).unwrap_or(usize::MAX);
    let has_more = local_more || collection.len() > limit;
    collection.truncate(limit);
    LivePage::new(
        ListPageResult {
            collection,
            page: 0,
            page_size: request.limit,
            has_more,
        },
        meta,
    )
}
