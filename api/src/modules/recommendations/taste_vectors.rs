use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use sqlx::PgPool;
use tracing::{debug, warn};

use super::clusters::recommend_id_str;
use super::debias::ips_debias;
use super::rerank_multi::RerankOptions;
use super::service::{RecommendResult, RecommendationsService};
use crate::common::sc_ids::user_id_variants;

const TASTE_DIMENSIONS: usize = 128;
const TASTE_POOL: usize = 300;
const TASTE_COLLECTION_PREFIX: &str = "tracks_taste_";
const READ_WARNING_EVERY_S: u64 = 60;

static LAST_READ_WARNING: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct UserTaste {
    pub version: String,
    pub collection: String,
    pub vector: Vec<f32>,
}

pub(crate) struct TastePool {
    pub version: String,
    pub candidates: Vec<RecommendResult>,
}

pub(crate) struct TasteShelf<'a> {
    pub taken: &'a HashSet<String>,
    pub per_cluster: usize,
    pub anti_centroid: Option<&'a [f32]>,
    pub recent_artists: &'a HashSet<String>,
    pub user_centroid: Option<&'a [f32]>,
}

pub(crate) async fn load_user_taste(pg: &PgPool, sc_user_id: &str) -> Option<UserTaste> {
    let account = account_number(sc_user_id)?;
    let row = match sqlx::query_file!("queries/recommendations/taste/user_vector.sql", account)
        .fetch_optional(pg)
        .await
    {
        Ok(row) => row?,
        Err(error) => {
            note_read_failure(&error);
            return None;
        }
    };
    usable(UserTaste {
        version: row.version,
        collection: row.collection,
        vector: row.vec,
    })
}

fn note_read_failure(error: &sqlx::Error) {
    crate::metrics::record_taste_vector_read_error();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let last = LAST_READ_WARNING.load(Ordering::Relaxed);
    let due = now.saturating_sub(last) >= READ_WARNING_EVERY_S
        && LAST_READ_WARNING
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok();
    if due {
        warn!(%error, "taste vectors cannot be read; home waves go without the taste shelf");
    } else {
        debug!(%error, "taste vector could not be read; the wave goes without it");
    }
}

fn usable(taste: UserTaste) -> Option<UserTaste> {
    let fits = taste.vector.len() == TASTE_DIMENSIONS
        && taste.vector.iter().all(|value| value.is_finite())
        && taste.vector.iter().any(|value| *value != 0.0)
        && taste.collection.starts_with(TASTE_COLLECTION_PREFIX);
    fits.then_some(taste)
}

fn account_number(sc_user_id: &str) -> Option<&str> {
    let trimmed = sc_user_id.trim();
    let bare = trimmed.rsplit(':').next().unwrap_or(trimmed);
    let numeric = !bare.is_empty() && bare.len() <= 18 && bare.bytes().all(|b| b.is_ascii_digit());
    numeric.then_some(bare)
}

impl RecommendationsService {
    pub(crate) async fn taste_pool(
        &self,
        sc_user_id: &str,
        exclude: &[String],
    ) -> Option<TastePool> {
        let taste = load_user_taste(&self.pg, sc_user_id).await?;
        let filter = self.build_filter(exclude, None);
        let mut candidates = self
            .search_by_vector(
                &taste.collection,
                &taste.vector,
                filter.as_ref(),
                TASTE_POOL,
            )
            .await;
        let owned = self.owned_tracks(sc_user_id, &candidates).await?;
        candidates.retain(|result| !owned.contains(&recommend_id_str(&result.id)));
        Some(TastePool {
            version: taste.version,
            candidates,
        })
    }

    async fn owned_tracks(
        &self,
        sc_user_id: &str,
        candidates: &[RecommendResult],
    ) -> Option<HashSet<String>> {
        let ids: Vec<String> = candidates
            .iter()
            .map(|result| recommend_id_str(&result.id))
            .filter(|id| !id.is_empty())
            .collect();
        if ids.is_empty() {
            return Some(HashSet::new());
        }
        match sqlx::query_file_scalar!(
            "queries/recommendations/taste/owned_tracks.sql",
            &user_id_variants(sc_user_id),
            &ids
        )
        .fetch_all(&self.pg)
        .await
        {
            Ok(owned) => Some(owned.into_iter().collect()),
            Err(error) => {
                warn!(%error, "the listener's own tracks are unknown; the taste shelf is left out");
                None
            }
        }
    }

    pub(crate) async fn build_taste_shelf(
        &self,
        pool: TastePool,
        shelf: TasteShelf<'_>,
    ) -> Vec<RecommendResult> {
        let mut pool = pool.candidates;
        pool.retain(|result| !shelf.taken.contains(&recommend_id_str(&result.id)));
        if pool.is_empty() {
            return Vec::new();
        }
        self.attach_playback_counts(&mut pool).await;
        ips_debias(&mut pool);
        self.rerank_multi(
            pool,
            RerankOptions {
                limit: shelf.per_cluster,
                diversity: 0.35,
                novelty: 0.15,
                serendipity: 0.05,
                anti_centroid: shelf.anti_centroid.map(<[f32]>::to_vec),
                recent_artists: shelf.recent_artists.clone(),
                user_centroid: shelf.user_centroid.map(<[f32]>::to_vec),
            },
        )
        .await
        .into_iter()
        .take(shelf.per_cluster)
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACTIVE: &str = "taste-202609241200-00000002";
    const RETIRED: &str = "taste-202609201200-00000001";

    fn axis(index: usize) -> Vec<f32> {
        let mut vector = vec![0.0; TASTE_DIMENSIONS];
        if let Some(slot) = vector.get_mut(index) {
            *slot = 1.0;
        }
        vector
    }

    async fn version(pg: &PgPool, version: &str, active: bool) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO taste_model_versions
                 (version, input_object, collection, trained_at, dim, pooling, metrics,
                  items_count, users_count, active)
             VALUES ($1, $1, 'tracks_taste_' || replace(substr($1, 7), '-', '_'), now(), 128,
                     '{}', '{}', 1, 1, $2)",
        )
        .bind(version)
        .bind(active)
        .execute(pg)
        .await?;
        Ok(())
    }

    async fn vector(pg: &PgPool, user: &str, version: &str, at: usize) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO user_taste_vectors (sc_user_id, version, vec) VALUES ($1, $2, $3)",
        )
        .bind(user)
        .bind(version)
        .bind(axis(at))
        .execute(pg)
        .await?;
        Ok(())
    }

    #[test]
    fn any_spelling_of_an_account_finds_the_same_row() {
        assert_eq!(account_number("soundcloud:users:42"), Some("42"));
        assert_eq!(account_number(" 42 "), Some("42"));
        assert_eq!(account_number("soundcloud:users:"), None);
        assert_eq!(account_number("someone"), None);
    }

    #[test]
    fn a_vector_that_cannot_live_in_the_taste_space_is_ignored() {
        let taste = UserTaste {
            version: ACTIVE.to_owned(),
            collection: "tracks_taste_202609241200_00000002".to_owned(),
            vector: axis(0),
        };

        assert!(usable(taste.clone()).is_some());
        assert!(
            usable(UserTaste {
                vector: vec![1.0; 64],
                ..taste.clone()
            })
            .is_none()
        );
        assert!(
            usable(UserTaste {
                vector: vec![0.0; TASTE_DIMENSIONS],
                ..taste.clone()
            })
            .is_none()
        );
        assert!(
            usable(UserTaste {
                collection: "tracks_mert".to_owned(),
                ..taste
            })
            .is_none()
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn only_a_vector_of_the_serving_version_is_used(pg: PgPool) -> anyhow::Result<()> {
        version(&pg, RETIRED, false).await?;
        version(&pg, ACTIVE, true).await?;
        vector(&pg, "42", RETIRED, 1).await?;
        vector(&pg, "42", ACTIVE, 2).await?;
        vector(&pg, "43", RETIRED, 3).await?;

        let served = load_user_taste(&pg, "soundcloud:users:42").await;
        let stale_only = load_user_taste(&pg, "43").await;
        let unknown = load_user_taste(&pg, "44").await;

        assert_eq!(
            served,
            Some(UserTaste {
                version: ACTIVE.to_owned(),
                collection: "tracks_taste_202609241200_00000002".to_owned(),
                vector: axis(2),
            })
        );
        assert_eq!(
            stale_only, None,
            "a vector from a retired model lives in another space"
        );
        assert_eq!(unknown, None);
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_broken_vector_table_is_counted_not_hidden(pg: PgPool) -> anyhow::Result<()> {
        crate::metrics::init();
        sqlx::query("DROP TABLE user_taste_vectors")
            .execute(&pg)
            .await?;

        let taste = load_user_taste(&pg, "42").await;

        assert_eq!(taste, None);
        if let Some(rendered) = crate::metrics::render_for_tests() {
            assert!(rendered.contains("api_taste_vector_read_errors_total"));
        }
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_listeners_own_tracks_are_found_among_the_candidates(
        pg: PgPool,
    ) -> anyhow::Result<()> {
        sqlx::raw_sql(
            "INSERT INTO user_likes_tracks (user_id, sc_track_id, wanted_state) VALUES
                ('42', 'soundcloud:tracks:1', true),
                ('soundcloud:users:42', '2', true),
                ('42', '3', false),
                ('43', '4', true);
             INSERT INTO user_events (sc_user_id, sc_track_id, event_type, weight, created_at)
             VALUES
                ('soundcloud:users:42', '5', 'playlist_add', 0.9, now() - interval '400 days'),
                ('42', 'soundcloud:tracks:6', 'full_play', 0.3, now()),
                ('42', '7', 'skip', -0.8, now()),
                ('42', '8', 'impression', 0.0, now()),
                ('43', '9', 'like', 1.0, now());",
        )
        .execute(&pg)
        .await?;
        let candidates: Vec<String> = (1..=10).map(|id| id.to_string()).collect();

        let mut owned: Vec<String> = sqlx::query_file_scalar!(
            "queries/recommendations/taste/owned_tracks.sql",
            &user_id_variants("soundcloud:users:42"),
            &candidates
        )
        .fetch_all(&pg)
        .await?;
        owned.sort();

        assert_eq!(owned, vec!["1", "2", "5", "6", "7"]);
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn without_any_trained_model_nobody_has_a_taste_vector(pg: PgPool) -> anyhow::Result<()> {
        assert_eq!(load_user_taste(&pg, "42").await, None);
        Ok(())
    }
}
