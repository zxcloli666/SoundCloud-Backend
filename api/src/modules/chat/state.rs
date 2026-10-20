use std::sync::Arc;

use deadpool_redis::Pool;
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;

use crate::modules::auth::{AuthService, LinkService};
use crate::modules::chat::ChatConfig;
use crate::modules::chat::channel::ock::OckRing;
use crate::modules::chat::hot_cache::HotCache;
use crate::modules::chat::hub::ChatHub;
use crate::modules::chat::kek::KekRing;
use crate::modules::chat::moderation::Moderator;
use crate::modules::rooms::RoomsService;

pub struct ChatState {
    pub config: ChatConfig,
    pub pg: PgPool,
    pub redis: Pool,
    pub ocks: OckRing,
    pub keks: KekRing,
    pub hub: ChatHub,
    pub hot: HotCache,
    pub moderator: Moderator,
    pub auth: Arc<AuthService>,
    pub link: Arc<LinkService>,
    pub rooms: Arc<RoomsService>,
    pub shutdown: CancellationToken,
}
