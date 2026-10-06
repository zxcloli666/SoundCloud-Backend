use std::sync::Arc;

use sqlx::PgPool;

use crate::background_jobs::{BackgroundJobs, CollabJobs};
use crate::cache::CacheService;
use crate::common::admission::PublicAdmission;
use crate::common::http_metrics::HttpMetrics;
use crate::config::AppConfig;
use crate::modules::auras::AurasService;
use crate::modules::auth::{AuthService, LinkService};
use crate::modules::collab::CollabVectorService;
use crate::modules::discover::DiscoverService;
use crate::modules::dislikes::DislikesService;
use crate::modules::events::EventsService;
use crate::modules::featured::FeaturedService;
use crate::modules::history::HistoryService;
use crate::modules::indexing::IndexingService;
use crate::modules::likes::LikesService;
use crate::modules::lyrics::LyricsService;
use crate::modules::me::MeService;
use crate::modules::oauth_apps::OAuthAppsService;
use crate::modules::playlists::PlaylistsService;
use crate::modules::recommendations::RecommendationsService;
use crate::modules::search::{SearchService, VibeSearchService};
use crate::modules::subscriptions::SubscriptionsService;
use crate::modules::sync_queue::SyncQueueService;
use crate::modules::tracks::TracksService;
use crate::modules::users::UsersService;
use crate::sc::ScReadService;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub pg: PgPool,
    pub background_jobs: Arc<BackgroundJobs>,
    pub http_metrics: Arc<HttpMetrics>,
    pub cache: Arc<CacheService>,
    pub auth: Arc<AuthService>,
    pub admission: Arc<PublicAdmission>,
    pub link: Arc<LinkService>,
    pub oauth_apps: Arc<OAuthAppsService>,
    pub events: Arc<EventsService>,
    pub dislikes: Arc<DislikesService>,
    pub subscriptions: Arc<SubscriptionsService>,
    pub auras: Arc<AurasService>,
    pub me: Arc<MeService>,
    pub tracks: Arc<TracksService>,
    pub playlists: Arc<PlaylistsService>,
    pub users: Arc<UsersService>,
    pub likes: Arc<LikesService>,
    pub resolve: Arc<ScReadService>,
    pub search: Arc<SearchService>,
    pub vibe: Arc<VibeSearchService>,
    pub history: Arc<HistoryService>,
    pub featured: Arc<FeaturedService>,
    pub lyrics: Arc<LyricsService>,
    pub collab_vector: Arc<CollabVectorService>,
    pub collab_jobs: Arc<CollabJobs>,
    pub indexing: Arc<IndexingService>,
    pub recommendations: Arc<RecommendationsService>,
    pub discover: Arc<DiscoverService>,
    pub sync_queue: Arc<SyncQueueService>,
}
