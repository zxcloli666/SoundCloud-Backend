use std::collections::HashMap;

use serde_json::{Map, Value};
use sqlx::PgPool;

use super::clusters::recommend_id_str;
use super::service::RecommendResult;
use crate::common::sc_ids::EntityRef;
use crate::error::AppResult;

pub(crate) async fn tracks_by_id(
    pg: &PgPool,
    sc_user_id: &str,
    ids: &[String],
) -> AppResult<HashMap<String, Value>> {
    let mut wanted: Vec<String> = Vec::new();
    for id in ids {
        if let Some(id) = crate::common::sc_ids::normalize_sc_track_id(id)
            && !wanted.contains(&id)
        {
            wanted.push(id);
        }
    }
    let mut tracks: Vec<Value> = crate::modules::tracks::project_many_public(pg, &wanted)
        .await?
        .into_iter()
        .flatten()
        .collect();
    crate::modules::enrich::dto::apply_to_tracks(pg, &mut tracks).await?;
    crate::modules::likes::cold::apply_user_favorite_flag(pg, sc_user_id, &mut tracks).await?;
    Ok(tracks
        .into_iter()
        .filter_map(|track| {
            let urn = track.get("urn")?.as_str()?;
            Some((EntityRef::track(urn)?.sc_id(), track))
        })
        .collect())
}

pub(crate) async fn results(
    pg: &PgPool,
    sc_user_id: &str,
    results: Vec<RecommendResult>,
) -> AppResult<Vec<Value>> {
    let ids: Vec<String> = results
        .iter()
        .map(|result| recommend_id_str(&result.id))
        .collect();
    let tracks = tracks_by_id(pg, sc_user_id, &ids).await?;
    Ok(results
        .into_iter()
        .zip(ids)
        .map(|(result, id)| {
            let mut value = serde_json::to_value(result).unwrap_or(Value::Null);
            if let Some(object) = value.as_object_mut() {
                attach(object, &id, &tracks);
            }
            value
        })
        .collect())
}

fn attach(object: &mut Map<String, Value>, id: &str, tracks: &HashMap<String, Value>) {
    if let Some(entity) = EntityRef::track(id) {
        object.insert("urn".into(), Value::String(entity.urn()));
    }
    if let Some(track) = EntityRef::track(id).and_then(|entity| tracks.get(&entity.sc_id())) {
        object.insert("track".into(), track.clone());
    }
}

pub(crate) async fn clusters(pg: &PgPool, sc_user_id: &str, json: &str) -> AppResult<String> {
    let Ok(mut response) = serde_json::from_str::<Value>(json) else {
        return Ok(json.to_owned());
    };
    let Some(clusters) = response.get_mut("clusters").and_then(Value::as_array_mut) else {
        return Ok(json.to_owned());
    };
    let ids: Vec<String> = clusters.iter().flat_map(cluster_ids).collect();
    let tracks = tracks_by_id(pg, sc_user_id, &ids).await?;
    for cluster in clusters.iter_mut() {
        let ids = cluster_ids(cluster);
        let Some(object) = cluster.as_object_mut() else {
            continue;
        };
        let urns: Vec<Value> = ids
            .iter()
            .filter_map(|id| EntityRef::track(id))
            .map(|entity| Value::String(entity.urn()))
            .collect();
        let projected: Vec<Value> = ids
            .iter()
            .filter_map(|id| EntityRef::track(id))
            .filter_map(|entity| tracks.get(&entity.sc_id()).cloned())
            .collect();
        object.insert("track_urns".into(), Value::Array(urns));
        object.insert("tracks".into(), Value::Array(projected));
    }
    Ok(serde_json::to_string(&response).unwrap_or_else(|_| json.to_owned()))
}

fn cluster_ids(cluster: &Value) -> Vec<String> {
    cluster
        .get("track_ids")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(recommend_id_str)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn each_result_gains_its_urn_and_only_a_known_track() {
        let tracks: HashMap<String, Value> =
            [("5".to_owned(), json!({"urn": "soundcloud:tracks:5"}))].into();
        let mut known = Map::new();
        attach(&mut known, "5", &tracks);
        assert_eq!(known["urn"], "soundcloud:tracks:5");
        assert_eq!(known["track"]["urn"], "soundcloud:tracks:5");

        let mut unknown = Map::new();
        attach(&mut unknown, "6", &tracks);
        assert_eq!(unknown["urn"], "soundcloud:tracks:6");
        assert!(!unknown.contains_key("track"));

        let mut broken = Map::new();
        attach(&mut broken, "", &tracks);
        assert!(broken.is_empty());
    }

    #[test]
    fn cluster_ids_read_numbers_and_strings_alike() {
        assert_eq!(
            cluster_ids(&json!({"track_ids": ["5", 6]})),
            vec!["5".to_owned(), "6".to_owned()]
        );
        assert!(cluster_ids(&json!({"id": "x"})).is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn clusters_keep_their_ids_and_gain_urns_and_projected_tracks(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, sharing)
             VALUES ('5', 'soundcloud:tracks:5', 'Five', 'five', 1000, 'public'),
                    ('6', 'soundcloud:tracks:6', 'Six', 'six', 1000, 'private')",
        )
        .execute(&pool)
        .await?;
        sqlx::query(
            "INSERT INTO user_likes_tracks (user_id, sc_track_id, wanted_state) VALUES ('17', '5', true)",
        )
        .execute(&pool)
        .await?;
        let json = json!({"clusters": [{"id": "wave", "track_ids": ["5", "6", "7"]}]}).to_string();

        let out: Value = serde_json::from_str(&clusters(&pool, "17", &json).await?)?;
        let cluster = &out["clusters"][0];
        assert_eq!(cluster["track_ids"], json!(["5", "6", "7"]));
        assert_eq!(
            cluster["track_urns"],
            json!([
                "soundcloud:tracks:5",
                "soundcloud:tracks:6",
                "soundcloud:tracks:7"
            ])
        );
        let tracks = cluster["tracks"].as_array().unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0]["urn"], "soundcloud:tracks:5");
        assert_eq!(tracks[0]["user_favorite"], true);
        assert!(tracks[0]["_scd_meta"].is_object());
        Ok(())
    }
}
