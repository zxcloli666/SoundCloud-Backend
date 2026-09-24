use super::collection::CollectionSync;
use crate::config::ColdCfg;
use crate::error::AppResult;
use crate::modules::indexing::IndexingService;
use backend_contracts::CatalogCollection;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::OnceCell;

#[derive(Debug, Clone, Copy)]
pub struct UserCollection {
    pub kind: CatalogCollection,
    pub lock_kind: &'static str,
    pub mirror_table: &'static str,
    pub mirror_key_col: &'static str,
    pub entity_kind: EntityKind,
    pub has_wanted_state: bool,
    pub order_by_release: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum EntityKind {
    Track,
    Playlist,
    User,
}

pub const LIKED_TRACKS: UserCollection = UserCollection {
    kind: CatalogCollection::LikedTracks,
    lock_kind: "liked-tracks",
    mirror_table: "user_likes_tracks",
    mirror_key_col: "sc_track_id",
    entity_kind: EntityKind::Track,
    has_wanted_state: true,
    order_by_release: false,
};

pub const LIKED_PLAYLISTS: UserCollection = UserCollection {
    kind: CatalogCollection::LikedPlaylists,
    lock_kind: "liked-playlists",
    mirror_table: "user_likes_playlists",
    mirror_key_col: "playlist_urn",
    entity_kind: EntityKind::Playlist,
    has_wanted_state: true,
    order_by_release: false,
};

pub const FOLLOWINGS: UserCollection = UserCollection {
    kind: CatalogCollection::Followings,
    lock_kind: "followings",
    mirror_table: "user_followings",
    mirror_key_col: "target_user_urn",
    entity_kind: EntityKind::User,
    has_wanted_state: true,
    order_by_release: false,
};

pub const FOLLOWERS: UserCollection = UserCollection {
    kind: CatalogCollection::Followers,
    lock_kind: "followers",
    mirror_table: "user_followers",
    mirror_key_col: "target_user_urn",
    entity_kind: EntityKind::User,
    has_wanted_state: false,
    order_by_release: false,
};

pub const OWNED_PLAYLISTS: UserCollection = UserCollection {
    kind: CatalogCollection::OwnedPlaylists,
    lock_kind: "owned-playlists",
    mirror_table: "user_owned_playlists",
    mirror_key_col: "playlist_urn",
    entity_kind: EntityKind::Playlist,
    has_wanted_state: false,
    order_by_release: false,
};

pub const OWNED_TRACKS: UserCollection = UserCollection {
    kind: CatalogCollection::OwnedTracks,
    lock_kind: "owned-tracks",
    mirror_table: "user_owned_tracks",
    mirror_key_col: "sc_track_id",
    entity_kind: EntityKind::Track,
    has_wanted_state: false,
    order_by_release: true,
};

#[derive(Debug, Clone, Copy)]
pub struct AudienceCollection {
    pub kind: CatalogCollection,
    pub subject: backend_contracts::CatalogEntity,
    pub ttl_sec: u64,
}

pub const TRACK_FAVORITERS: AudienceCollection = AudienceCollection {
    kind: CatalogCollection::TrackFavoriters,
    subject: backend_contracts::CatalogEntity::Track,
    ttl_sec: 600,
};

pub const TRACK_REPOSTERS: AudienceCollection = AudienceCollection {
    kind: CatalogCollection::TrackReposters,
    subject: backend_contracts::CatalogEntity::Track,
    ttl_sec: 600,
};

pub const PLAYLIST_REPOSTERS: AudienceCollection = AudienceCollection {
    kind: CatalogCollection::PlaylistReposters,
    subject: backend_contracts::CatalogEntity::Playlist,
    ttl_sec: 600,
};

impl AudienceCollection {
    pub fn subject_urn(&self, urn: &str) -> String {
        self.subject.urn(crate::common::sc_ids::extract_sc_id(urn))
    }

    pub fn sibling_relations(&self) -> Vec<String> {
        match self.subject {
            backend_contracts::CatalogEntity::Playlist => vec![PLAYLIST_REPOSTERS.kind],
            _ => vec![TRACK_FAVORITERS.kind, TRACK_REPOSTERS.kind],
        }
        .into_iter()
        .map(|kind| kind.as_str().to_owned())
        .collect()
    }
}

pub struct ColdRefreshService {
    pg: PgPool,
    cfg: ColdCfg,
    indexing: OnceCell<Arc<IndexingService>>,
}

impl ColdRefreshService {
    pub fn new(pg: PgPool, cfg: ColdCfg) -> Arc<Self> {
        Arc::new(Self {
            pg,
            cfg,
            indexing: OnceCell::new(),
        })
    }

    pub fn install_indexing(&self, indexing: Arc<IndexingService>) {
        let _ = self.indexing.set(indexing);
    }

    pub fn indexing_for_ingest(&self) -> Option<&Arc<IndexingService>> {
        self.indexing.get()
    }

    pub fn is_track_stale(&self, synced_at: Option<DateTime<Utc>>) -> bool {
        is_stale(synced_at, self.cfg.track_ttl_sec)
    }

    pub fn is_user_stale(&self, synced_at: Option<DateTime<Utc>>) -> bool {
        is_stale(synced_at, self.cfg.user_ttl_sec)
    }

    pub fn is_playlist_stale(&self, synced_at: Option<DateTime<Utc>>) -> bool {
        is_stale(synced_at, self.cfg.playlist_ttl_sec)
    }

    fn ttl_for(&self, coll: &UserCollection) -> u64 {
        match coll.lock_kind {
            k if k == LIKED_TRACKS.lock_kind => self.cfg.liked_tracks_ttl_sec,
            k if k == LIKED_PLAYLISTS.lock_kind => self.cfg.liked_playlists_ttl_sec,
            k if k == FOLLOWINGS.lock_kind => self.cfg.followings_ttl_sec,
            k if k == FOLLOWERS.lock_kind => 600,
            _ => self.cfg.owned_ttl_sec,
        }
    }

    pub async fn ensure_collection(
        &self,
        coll: UserCollection,
        sc_user_id: &str,
        viewer_is_owner: bool,
    ) -> AppResult<CollectionSync> {
        super::collection::ensure(
            &self.pg,
            coll.kind,
            sc_user_id,
            viewer_is_owner,
            self.ttl_for(&coll),
        )
        .await
    }

    pub async fn audience_page(
        &self,
        coll: AudienceCollection,
        subject_urn: &str,
        page: i64,
        limit: i64,
    ) -> AppResult<super::collection::CollectionPage> {
        let subject = coll.subject_urn(subject_urn);
        let Some(public) = self.subject_is_public(coll.subject, &subject).await? else {
            super::entity::enqueue_entity(&self.pg, coll.subject, &subject, None).await?;
            return Ok(super::collection::CollectionPage::empty(
                refreshing(),
                page,
                limit,
            ));
        };
        if !public {
            sqlx::query_file!(
                "queries/cold_refresh/audience_clear.sql",
                &subject,
                crate::common::sc_ids::extract_sc_id(&subject),
                &coll.sibling_relations()
            )
            .execute(&self.pg)
            .await?;
            return Ok(super::collection::CollectionPage::empty(
                ready(),
                page,
                limit,
            ));
        }
        let sync =
            super::collection::ensure(&self.pg, coll.kind, &subject, false, coll.ttl_sec).await?;
        let page = super::read_audience_page(&self.pg, &coll, &subject, page, limit).await?;
        Ok(super::collection::CollectionPage::new(page, sync))
    }

    pub async fn comments_page(
        &self,
        sc_track_id: &str,
        page: i64,
        limit: i64,
    ) -> AppResult<super::collection::CollectionPage> {
        let subject = backend_contracts::CatalogEntity::Track.urn(sc_track_id);
        let Some(public) = self
            .subject_is_public(backend_contracts::CatalogEntity::Track, &subject)
            .await?
        else {
            super::entity::enqueue_entity(
                &self.pg,
                backend_contracts::CatalogEntity::Track,
                &subject,
                None,
            )
            .await?;
            return Ok(super::collection::CollectionPage::empty(
                refreshing(),
                page,
                limit,
            ));
        };
        if !public {
            sqlx::query_file!("queries/cold_refresh/comments_clear.sql", sc_track_id)
                .execute(&self.pg)
                .await?;
            return Ok(super::collection::CollectionPage::empty(
                ready(),
                page,
                limit,
            ));
        }
        let sync = super::collection::ensure(
            &self.pg,
            CatalogCollection::TrackComments,
            &subject,
            false,
            COMMENTS_TTL_SEC,
        )
        .await?;
        let page =
            crate::modules::tracks::comments::read_page(&self.pg, sc_track_id, page, limit).await?;
        Ok(super::collection::CollectionPage::new(page, sync))
    }

    async fn subject_is_public(
        &self,
        subject: backend_contracts::CatalogEntity,
        subject_urn: &str,
    ) -> AppResult<Option<bool>> {
        let id = crate::common::sc_ids::extract_sc_id(subject_urn);
        Ok(match subject {
            backend_contracts::CatalogEntity::Playlist => {
                sqlx::query_file_scalar!("queries/cold_refresh/playlist_is_public.sql", subject_urn)
                    .fetch_optional(&self.pg)
                    .await?
            }
            _ => {
                sqlx::query_file_scalar!("queries/cold_refresh/track_is_public.sql", id)
                    .fetch_optional(&self.pg)
                    .await?
            }
        })
    }
}

const COMMENTS_TTL_SEC: u64 = 600;

fn refreshing() -> CollectionSync {
    CollectionSync {
        status: "refreshing",
        last_completed_at: None,
        retry_after_seconds: 5,
    }
}

fn ready() -> CollectionSync {
    CollectionSync {
        status: "ready",
        last_completed_at: None,
        retry_after_seconds: 0,
    }
}

fn is_stale(synced_at: Option<DateTime<Utc>>, ttl_sec: u64) -> bool {
    match synced_at {
        None => true,
        Some(t) => {
            let age = Utc::now().signed_duration_since(t).num_seconds();
            age < 0 || age as u64 > ttl_sec
        }
    }
}
