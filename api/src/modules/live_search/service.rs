use std::collections::HashMap;
use std::convert::Infallible;
use std::future::Future;
use std::sync::{Arc, OnceLock};

use axum::http::HeaderMap;
use serde_json::Value;
use sqlx::PgPool;
use tracing::debug;

use super::fetch::{Fetch, Fetched};
use super::gate::{Closed, LiveGate};
use super::merge::{self, SOURCE_LOCAL};
use super::meta::{LiveMeta, LivePage, LiveState};
use super::query::{
    ATTACH_CAP, FAIL_TTL, IMPORT_LOCAL_ROWS, Intent, LOCAL_CAP, LOCAL_PATIENCE, LiveClass,
    LiveKind, LiveQuery, class_of, identity_of,
};
use super::serving;
use super::slim::{self, urn_of};
use super::store::{LiveStore, Window};
use crate::cache::{CacheService, KeyedCoalesce, ListPageResult};
use crate::common::admission::PublicAdmission;
use crate::common::pagination::PaginationQuery;
use crate::config::{LiveMode, LiveSearchCfg};
use crate::error::AppResult;
use crate::modules::enrich::dto as enrich_dto;
use crate::modules::indexing::IndexingService;
use crate::modules::search::failure;
use crate::modules::search::service::MAX_LIMIT;
use crate::sc::{PgEgressHealth, ScReadService};

#[path = "service_import.rs"]
mod import;

type LocalPage = AppResult<ListPageResult<Value>>;

#[derive(Clone, Debug)]
pub struct LiveRequest {
    pub kind: LiveKind,
    pub class: LiveClass,
    pub query: LiveQuery,
    pub page: i64,
    pub limit: i64,
    pub local_limit: i64,
    pub identity: String,
}

impl LiveRequest {
    fn scope(&self) -> &'static str {
        self.class.scope(self.kind)
    }
}

#[derive(Clone, Debug)]
enum Outcome {
    Window {
        window: Window,
        items: Vec<Value>,
        state: LiveState,
    },
    Missed(Closed),
}

struct Served {
    window: Option<Window>,
    items: Vec<Value>,
    meta: LiveMeta,
}

impl Served {
    fn window(window: Window, items: Vec<Value>, state: LiveState) -> Self {
        Self {
            window: Some(window),
            items,
            meta: LiveMeta::new(state, None),
        }
    }

    fn nothing(state: LiveState, retry_after: Option<i64>) -> Self {
        Self {
            window: None,
            items: Vec::new(),
            meta: LiveMeta::new(state, retry_after),
        }
    }
}

pub struct LiveSearch {
    read: Arc<ScReadService>,
    store: LiveStore,
    gate: LiveGate,
    flights: KeyedCoalesce<Outcome>,
    pg: PgPool,
    indexing: OnceLock<Arc<IndexingService>>,
    cfg: LiveSearchCfg,
}

impl LiveSearch {
    pub fn new(
        read: Arc<ScReadService>,
        cache: Arc<CacheService>,
        admission: Arc<PublicAdmission>,
        pg: PgPool,
        cfg: LiveSearchCfg,
    ) -> Arc<Self> {
        Arc::new(Self {
            read,
            store: LiveStore::new(cache),
            gate: LiveGate::new(
                PgEgressHealth::new(pg.clone()),
                admission,
                cfg.max_in_flight,
            ),
            flights: KeyedCoalesce::new(),
            pg,
            indexing: OnceLock::new(),
            cfg,
        })
    }

    pub fn install_indexing(&self, indexing: Arc<IndexingService>) {
        let _ = self.indexing.set(indexing);
    }

    pub fn plan(
        &self,
        kind: LiveKind,
        raw: Option<&str>,
        headers: &HeaderMap,
        pagination: &PaginationQuery,
        linked_partitioning: bool,
        sc_user_id: &str,
    ) -> Option<LiveRequest> {
        let intent = Intent::from_headers(headers);
        if self.cfg.mode == LiveMode::Off && intent == Intent::Absent {
            return None;
        }
        let query = LiveQuery::parse(raw?)?;
        let (page, limit) = pagination.resolved();
        let limit = limit.min(MAX_LIMIT);
        let class = class_of(
            kind,
            intent,
            limit,
            pagination.page.is_some(),
            linked_partitioning,
        );
        let local_limit = if class == LiveClass::Import && self.cfg.ranked {
            limit.max(IMPORT_LOCAL_ROWS)
        } else {
            limit
        };
        Some(LiveRequest {
            kind,
            class,
            query,
            page,
            limit,
            local_limit,
            identity: identity_of(sc_user_id),
        })
    }

    pub fn rescue_plan(
        &self,
        raw: Option<&str>,
        headers: &HeaderMap,
        pagination: &PaginationQuery,
        sc_user_id: &str,
    ) -> Option<LiveRequest> {
        let wanted = LiveClass::Rescue.allowed_in(self.cfg.mode, self.cfg.db_rescue)
            && Intent::from_headers(headers) == Intent::Absent
            && pagination.page() == 0;
        if !wanted {
            return None;
        }
        let query = LiveQuery::parse(raw?).filter(LiveQuery::is_specific)?;
        let limit = pagination.limit().min(MAX_LIMIT);
        Some(LiveRequest {
            kind: LiveKind::Tracks,
            class: LiveClass::Rescue,
            query,
            page: 0,
            limit,
            local_limit: limit,
            identity: identity_of(sc_user_id),
        })
    }

    pub async fn page<F, Fut>(&self, request: &LiveRequest, local: F) -> AppResult<LivePage>
    where
        F: Fn(i64) -> Fut,
        Fut: Future<Output = LocalPage>,
    {
        let page = if !request.class.allowed_in(self.cfg.mode, self.cfg.db_rescue) {
            self.local_answer(
                request,
                bounded(local(request.page)).await?,
                LiveMeta::new(LiveState::Off, None),
            )
            .await
        } else if request.page == 0 {
            self.first_page(request, &local).await?
        } else {
            self.later_page(request, &local).await?
        };
        crate::metrics::record_live_request(
            request.kind.as_str(),
            request.class.as_str(),
            page.state().as_str(),
        );
        Ok(page)
    }

    async fn first_page<F, Fut>(&self, request: &LiveRequest, local: &F) -> AppResult<LivePage>
    where
        F: Fn(i64) -> Fut,
        Fut: Future<Output = LocalPage>,
    {
        let previous = self
            .store
            .window(request.scope(), &request.query.hash)
            .await;
        if let Some(window) = previous.as_ref().filter(|window| window.is_fresh(now())) {
            let served = Served::window(window.clone(), Vec::new(), LiveState::Cached);
            return self.serve(request, served, None, local).await;
        }
        let mut pending = std::pin::pin!(bounded(local(0)));
        let landed = match tokio::time::timeout(LOCAL_PATIENCE, &mut pending).await {
            Ok(Ok(page)) if self.local_is_enough(request, &page.collection) => {
                let meta = LiveMeta::new(LiveState::Local, None);
                return Ok(self.local_answer(request, page, meta).await);
            }
            Ok(result) => Some(result),
            Err(_) => None,
        };
        let (outcome, landed) = match landed {
            Some(result) => (self.live(request).await, result),
            None => tokio::join!(self.live(request), pending),
        };
        let served = served_from(outcome, previous);
        self.serve(request, served, Some(landed), local).await
    }

    async fn later_page<F, Fut>(&self, request: &LiveRequest, local: &F) -> AppResult<LivePage>
    where
        F: Fn(i64) -> Fut,
        Fut: Future<Output = LocalPage>,
    {
        let served = match self
            .store
            .window(request.scope(), &request.query.hash)
            .await
        {
            Some(window) if window.is_fresh(now()) => {
                Served::window(window, Vec::new(), LiveState::Cached)
            }
            Some(window) => Served::window(window, Vec::new(), LiveState::Stale),
            None => Served::nothing(LiveState::Skipped, None),
        };
        self.serve(request, served, None, local).await
    }

    async fn serve<F, Fut>(
        &self,
        request: &LiveRequest,
        served: Served,
        landed: Option<LocalPage>,
        local: &F,
    ) -> AppResult<LivePage>
    where
        F: Fn(i64) -> Fut,
        Fut: Future<Output = LocalPage>,
    {
        let Served {
            window,
            items,
            meta,
        } = served;
        let local_failed = matches!(landed, Some(Err(_)));
        let local_page_at = |index: i64, landed: Option<LocalPage>| async move {
            match landed {
                Some(result) if index == 0 => result,
                _ => bounded(local(index)).await,
            }
        };
        let Some(window) = window else {
            let page = local_page_at(request.page, landed).await?;
            return Ok(self.local_answer(request, page, meta).await);
        };
        if request.class == LiveClass::Import && request.page == 0 {
            let (hits, after) = tokio::join!(
                self.build(request, &window.ids, 0, &items),
                local_page_at(0, landed)
            );
            return Ok(self.import_page(request, hits, after, meta).await);
        }
        let wpages = merge::wpages(window.ids.len(), request.limit);
        if request.page >= wpages {
            let mut after = local_page_at(request.page - wpages, landed).await?;
            after.collection = merge::local_after_window(after.collection, &window.ids);
            after.page = request.page;
            return Ok(local_page(after, meta));
        }
        let slice = merge::window_slice(&window.ids, request.page, request.limit);
        let offset = usize::try_from(request.page.saturating_mul(request.limit)).unwrap_or(0);
        let meta = if local_failed {
            meta.local_unavailable()
        } else {
            meta
        };
        let (collection, has_more, meta) = if request.page + 1 < wpages {
            let collection = self.build(request, slice, offset, &items).await;
            (collection, true, meta)
        } else {
            let (collection, after) = tokio::join!(
                self.build(request, slice, offset, &items),
                local_page_at(0, landed)
            );
            match after {
                Ok(after) => {
                    let more = after.has_more
                        || !merge::local_after_window(after.collection, &window.ids).is_empty();
                    (collection, more, meta)
                }
                Err(_) => (collection, false, meta.local_unavailable()),
            }
        };
        Ok(LivePage::new(
            ListPageResult {
                collection,
                page: request.page,
                page_size: request.limit,
                has_more,
            },
            meta,
        ))
    }

    fn local_is_enough(&self, request: &LiveRequest, rows: &[Value]) -> bool {
        if request.class == LiveClass::Import && self.cfg.ranked {
            return merge::confident(&request.query.text, rows);
        }
        merge::local_is_enough(request.kind, &request.query.text, rows, self.cfg.ranked)
    }

    async fn local_answer(
        &self,
        request: &LiveRequest,
        page: ListPageResult<Value>,
        meta: LiveMeta,
    ) -> LivePage {
        if request.class == LiveClass::Import && self.cfg.ranked {
            return self.import_page(request, Vec::new(), Ok(page), meta).await;
        }
        local_page(page, meta)
    }

    async fn build(
        &self,
        request: &LiveRequest,
        ids: &[String],
        offset: usize,
        carried: &[Value],
    ) -> Vec<Value> {
        let kind = request.kind;
        let items = self.window_items(ids, carried).await;
        let urns: Vec<String> = items
            .iter()
            .filter_map(|item| urn_of(item).map(str::to_owned))
            .collect();
        let serving =
            match tokio::time::timeout(ATTACH_CAP, serving::lookup(&self.pg, kind, &urns)).await {
                Ok(Ok(serving)) => serving,
                Ok(Err(error)) => {
                    debug!(%error, "live hits are served without their local rows");
                    serving::Serving::new()
                }
                Err(_) => {
                    debug!("looking up local rows for live hits timed out");
                    serving::Serving::new()
                }
            };
        let mut collection = merge::substitute(items, &serving);
        if kind == LiveKind::Tracks {
            let enriched = tokio::time::timeout(
                ATTACH_CAP,
                enrich_dto::apply_to_tracks(&self.pg, &mut collection),
            )
            .await;
            if !matches!(enriched, Ok(Ok(()))) {
                debug!("live hits are served without enrichment");
            }
            if self.cfg.ranked {
                merge::mark_scores(&request.query.text, &mut collection, offset);
            }
        }
        collection
    }

    async fn window_items(&self, ids: &[String], carried: &[Value]) -> Vec<Value> {
        let carried: HashMap<&str, &Value> = carried
            .iter()
            .filter_map(|item| Some((urn_of(item)?, item)))
            .collect();
        let missing: Vec<String> = ids
            .iter()
            .filter(|id| !carried.contains_key(id.as_str()))
            .cloned()
            .collect();
        let mut stored: HashMap<String, Value> = missing
            .iter()
            .cloned()
            .zip(self.store.items(&missing).await)
            .filter_map(|(id, item)| Some((id, item?)))
            .collect();
        ids.iter()
            .filter_map(|id| match carried.get(id.as_str()) {
                Some(item) => Some((*item).clone()),
                None => stored.remove(id),
            })
            .collect()
    }

    async fn live(&self, request: &LiveRequest) -> Outcome {
        let key = format!("{}:{}", request.scope(), request.query.hash);
        let Ok(outcome) = self
            .flights
            .run(&key, || async {
                Ok::<_, Infallible>(self.lead(request).await)
            })
            .await;
        outcome
    }

    async fn lead(&self, request: &LiveRequest) -> Outcome {
        let (scope, hash) = (request.scope(), request.query.hash.as_str());
        let started = now();
        if let Some(window) = self
            .store
            .window(scope, hash)
            .await
            .filter(|window| window.is_fresh(started))
        {
            return Outcome::Window {
                window,
                items: Vec::new(),
                state: LiveState::Cached,
            };
        }
        if let Some(left) = self.store.failed(scope, hash, started).await {
            return Outcome::Missed(Closed::new(LiveState::Unavailable, left));
        }
        let permit = match self.gate.open(request.class, &request.identity).await {
            Ok(permit) => permit,
            Err(closed) => return Outcome::Missed(closed),
        };
        let fetched = Fetch {
            read: &self.read,
            gate: &self.gate,
            kind: request.kind,
            class: request.class,
            query: &request.query,
        }
        .run()
        .await;
        drop(permit);
        match fetched {
            Fetched::Items(raw) => self.keep(request, &raw).await,
            Fetched::Empty => self.keep(request, &[]).await,
            Fetched::RateLimited(retry_after) => {
                let seconds = self.gate.cool_down(retry_after).await;
                Outcome::Missed(Closed::new(LiveState::Cooling, seconds))
            }
            Fetched::Unavailable | Fetched::Timeout => {
                self.store.mark_failed(scope, hash, now()).await;
                self.gate.record_silence().await;
                let state = if fetched == Fetched::Timeout {
                    LiveState::Timeout
                } else {
                    LiveState::Unavailable
                };
                Outcome::Missed(Closed::new(state, FAIL_TTL as i64))
            }
            Fetched::Untried => Outcome::Missed(Closed {
                state: LiveState::Unavailable,
                retry_after: None,
            }),
        }
    }

    async fn keep(&self, request: &LiveRequest, raw: &[Value]) -> Outcome {
        let slimmed = slim::slim(request.kind, raw);
        let ids: Vec<String> = slimmed
            .items
            .iter()
            .filter_map(|item| urn_of(item).map(str::to_owned))
            .collect();
        let window = Window::new(ids, now());
        let ttl = request.class.window_ttl(window.ids.is_empty());
        let written = self
            .store
            .write(
                request.scope(),
                &request.query.hash,
                &window,
                ttl,
                &slimmed.items,
                &slimmed.users,
            )
            .await;
        crate::metrics::record_live_window_bytes(written);
        self.gate.record_answer().await;
        Outcome::Window {
            window,
            items: slimmed.items,
            state: LiveState::Fresh,
        }
    }
}

fn served_from(outcome: Outcome, previous: Option<Window>) -> Served {
    match (outcome, previous) {
        (
            Outcome::Window {
                window,
                items,
                state,
            },
            _,
        ) => Served::window(window, items, state),
        (Outcome::Missed(_), Some(window)) => Served::window(window, Vec::new(), LiveState::Stale),
        (Outcome::Missed(closed), None) => Served::nothing(closed.state, closed.retry_after),
    }
}

fn local_page(mut page: ListPageResult<Value>, meta: LiveMeta) -> LivePage {
    merge::tag_all(&mut page.collection, SOURCE_LOCAL);
    LivePage::new(page, meta)
}

async fn bounded<Fut>(local: Fut) -> LocalPage
where
    Fut: Future<Output = LocalPage>,
{
    tokio::time::timeout(LOCAL_CAP, local)
        .await
        .unwrap_or_else(|_| Err(failure::timed_out()))
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
