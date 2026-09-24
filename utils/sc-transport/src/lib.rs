mod apiv2;
mod channel_health;
mod client;
mod egress_health;
mod error;
mod lua_methods;
mod mapping;
mod pagination;
mod types;

#[derive(Clone, Debug)]
pub struct ScConfig {
    pub proxy_url: String,
    pub proxy_fallback: bool,
    pub api_base: Option<String>,
    pub home_base: Option<String>,
}

pub use apiv2::Apiv2Proxy;
pub use bytes::Bytes;
pub use channel_health::{ChannelHealth, Trip};
pub use client::{OAuthCredentials, RelayTransport, ScClient};
pub use egress_health::{
    EGRESS_RELAY_LUA, EGRESS_RELAY_RAW, EgressFuture, EgressHealth, EgressHealthStore, EgressState,
};
pub use error::{ScError, ScResult};
pub use mapping::{PublicCollection, SearchType, normalize_v2_to_v1, unwrap_collection_items};
pub use pagination::{Page, parse_list_cursor, parse_list_page};
pub use types::*;
