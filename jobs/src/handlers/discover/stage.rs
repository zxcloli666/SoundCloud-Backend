use chrono::Utc;
use sqlx::{Postgres, Transaction};

const INTERNAL_PLAY_WEIGHT: i64 = 10_000;

#[derive(Clone, Copy)]
pub(super) struct Shards {
    pub(super) count: i64,
    pub(super) current: i64,
}

impl Shards {
    pub(super) fn at(count: i64, hour: i64) -> Self {
        let count = count.max(1);
        Self {
            count,
            current: hour.rem_euclid(count),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Stage {
    ArtistCounts,
    ArtistPlays,
    ArtistPopularity,
    ArtistTags,
    TagCounts,
    ArtistStarActive,
    ArtistStarPremium,
    AlbumMeta,
    AlbumPopularity,
    AlbumStar,
}

#[derive(Clone, Copy)]
pub(crate) enum Group {
    Summary,
    ArtistPopularity,
    ArtistTags,
    Albums,
}

impl Group {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Summary => "summary",
            Self::ArtistPopularity => "artist_popularity",
            Self::ArtistTags => "artist_tags",
            Self::Albums => "albums",
        }
    }

    pub(crate) fn plan(self, always_premium: bool) -> Vec<Stage> {
        match self {
            Self::Summary => vec![
                Stage::ArtistCounts,
                Stage::ArtistPlays,
                if always_premium {
                    Stage::ArtistStarPremium
                } else {
                    Stage::ArtistStarActive
                },
                Stage::AlbumStar,
            ],
            Self::ArtistPopularity => vec![Stage::ArtistPopularity],
            Self::ArtistTags => vec![Stage::ArtistTags, Stage::TagCounts],
            Self::Albums => vec![Stage::AlbumMeta, Stage::AlbumPopularity],
        }
    }
}

impl Stage {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::ArtistCounts => "artist_counts",
            Self::ArtistPlays => "artist_plays",
            Self::ArtistPopularity => "artist_popularity",
            Self::ArtistTags => "artist_tags",
            Self::TagCounts => "tag_counts",
            Self::ArtistStarActive => "artist_star_active",
            Self::ArtistStarPremium => "artist_star_premium",
            Self::AlbumMeta => "album_meta",
            Self::AlbumPopularity => "album_popularity",
            Self::AlbumStar => "album_star",
        }
    }

    pub(super) async fn execute(
        self,
        transaction: &mut Transaction<'_, Postgres>,
        shards: Shards,
    ) -> Result<(), sqlx::Error> {
        match self {
            Self::ArtistCounts => {
                sqlx::query_file!("queries/discover/refresh_artist_counts.sql")
                    .execute(&mut **transaction)
                    .await?;
            }
            Self::ArtistPlays => {
                sqlx::query_file!(
                    "queries/discover/refresh_artist_plays.sql",
                    shards.count,
                    shards.current
                )
                .execute(&mut **transaction)
                .await?;
            }
            Self::ArtistPopularity => {
                sqlx::query_file!(
                    "queries/discover/refresh_artist_popularity.sql",
                    INTERNAL_PLAY_WEIGHT
                )
                .execute(&mut **transaction)
                .await?;
            }
            Self::ArtistTags => {
                sqlx::query_file!("queries/discover/refresh_artist_tags.sql")
                    .execute(&mut **transaction)
                    .await?;
            }
            Self::TagCounts => {
                sqlx::query_file!("queries/discover/refresh_tag_counts.sql")
                    .execute(&mut **transaction)
                    .await?;
            }
            Self::ArtistStarActive => {
                sqlx::query_file!(
                    "queries/discover/refresh_artist_star_active.sql",
                    Utc::now().timestamp()
                )
                .execute(&mut **transaction)
                .await?;
            }
            Self::ArtistStarPremium => {
                sqlx::query_file!("queries/discover/refresh_artist_star_premium.sql")
                    .execute(&mut **transaction)
                    .await?;
            }
            Self::AlbumMeta => {
                sqlx::query_file!("queries/discover/refresh_album_meta.sql")
                    .execute(&mut **transaction)
                    .await?;
            }
            Self::AlbumPopularity => {
                sqlx::query_file!("queries/discover/refresh_album_popularity.sql")
                    .execute(&mut **transaction)
                    .await?;
            }
            Self::AlbumStar => {
                sqlx::query_file!("queries/discover/refresh_album_star.sql")
                    .execute(&mut **transaction)
                    .await?;
            }
        }
        Ok(())
    }
}
