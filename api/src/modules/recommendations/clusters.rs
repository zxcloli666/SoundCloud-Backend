use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use super::service::RecommendResult;

#[derive(Debug, Serialize)]
pub struct ClusterResponse {
    pub clusters: Vec<Cluster>,
    #[serde(skip)]
    observations: HashMap<String, RecommendationObservation>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RecommendationObservation {
    pub score: Option<f32>,
    pub features: Option<Vec<f32>>,
}

#[derive(Debug, Serialize)]
pub struct Cluster {
    pub id: &'static str,
    pub track_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub neighbors: Option<Vec<ClusterNeighbor>>,
}

#[derive(Debug, Serialize)]
pub struct ClusterNeighbor {
    pub track_id: String,
    pub artist_id: Uuid,
    pub artist_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

pub struct ClusterBuilder {
    taken: HashSet<String>,
    shown: HashSet<String>,
    clusters: Vec<Cluster>,
    observations: HashMap<String, RecommendationObservation>,
}

impl ClusterBuilder {
    pub fn new() -> Self {
        Self {
            taken: HashSet::new(),
            shown: HashSet::new(),
            clusters: Vec::new(),
            observations: HashMap::new(),
        }
    }

    pub fn reserve(&mut self, ids: impl IntoIterator<Item = String>) {
        for id in ids {
            self.taken.insert(id);
        }
    }

    pub fn taken(&self) -> &HashSet<String> {
        &self.taken
    }

    pub fn push(&mut self, id: &'static str, track_ids: Vec<String>) {
        self.put(id, track_ids, &self.taken.clone());
    }

    pub fn push_reserved(&mut self, id: &'static str, track_ids: Vec<String>) {
        self.put(id, track_ids, &self.shown.clone());
    }

    fn put(&mut self, id: &'static str, track_ids: Vec<String>, refuse: &HashSet<String>) {
        let mut on_this_shelf: HashSet<String> = HashSet::new();
        let track_ids: Vec<String> = track_ids
            .into_iter()
            .filter(|track_id| !refuse.contains(track_id) && on_this_shelf.insert(track_id.clone()))
            .collect();
        if track_ids.is_empty() {
            return;
        }
        for t in &track_ids {
            self.taken.insert(t.clone());
            self.shown.insert(t.clone());
        }
        self.clusters.push(Cluster {
            id,
            track_ids,
            neighbors: None,
        });
    }

    pub fn push_observed(
        &mut self,
        id: &'static str,
        track_ids: Vec<String>,
        results: &[RecommendResult],
    ) {
        for track_id in &track_ids {
            if self.taken.contains(track_id) {
                continue;
            }
            let Some(result) = results
                .iter()
                .find(|result| recommend_id_str(&result.id) == *track_id)
            else {
                continue;
            };
            self.observations
                .insert(track_id.clone(), observation(result));
        }
        self.push(id, track_ids);
    }

    pub fn push_with_neighbors(&mut self, id: &'static str, neighbors: Vec<ClusterNeighbor>) {
        let mut on_this_shelf: HashSet<String> = HashSet::new();
        let neighbors: Vec<ClusterNeighbor> = neighbors
            .into_iter()
            .filter(|neighbor| {
                !self.taken.contains(&neighbor.track_id)
                    && on_this_shelf.insert(neighbor.track_id.clone())
            })
            .collect();
        if neighbors.is_empty() {
            return;
        }
        let track_ids: Vec<String> = neighbors.iter().map(|n| n.track_id.clone()).collect();
        for t in &track_ids {
            self.taken.insert(t.clone());
            self.shown.insert(t.clone());
        }
        self.clusters.push(Cluster {
            id,
            track_ids,
            neighbors: Some(neighbors),
        });
    }

    pub fn finish(self) -> ClusterResponse {
        ClusterResponse {
            clusters: self.clusters,
            observations: self.observations,
        }
    }

    pub fn all_track_ids(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for c in &self.clusters {
            for id in &c.track_ids {
                out.push(id.clone());
            }
        }
        out
    }

    pub fn drop_missing(&mut self, missing: &HashSet<String>) {
        if missing.is_empty() {
            return;
        }
        let mut kept: Vec<Cluster> = Vec::with_capacity(self.clusters.len());
        for c in self.clusters.drain(..) {
            let track_ids: Vec<String> = c
                .track_ids
                .into_iter()
                .filter(|id| !missing.contains(id))
                .collect();
            if track_ids.is_empty() {
                continue;
            }
            let neighbors = c.neighbors.map(|ns| {
                ns.into_iter()
                    .filter(|n| !missing.contains(&n.track_id))
                    .collect::<Vec<_>>()
            });
            kept.push(Cluster {
                id: c.id,
                track_ids,
                neighbors: neighbors.filter(|ns| !ns.is_empty()),
            });
        }
        self.clusters = kept;
        for id in missing.iter() {
            self.observations.remove(id);
        }
    }
}

impl ClusterResponse {
    pub(crate) fn observation(&self, track_id: &str) -> Option<&RecommendationObservation> {
        self.observations.get(track_id)
    }
}

impl Default for ClusterBuilder {
    fn default() -> Self {
        Self::new()
    }
}

pub fn recommend_id_str(v: &Value) -> String {
    if let Some(s) = v.as_str() {
        return s.to_string();
    }
    if let Some(n) = v.as_u64() {
        return n.to_string();
    }
    String::new()
}

fn observation(result: &RecommendResult) -> RecommendationObservation {
    RecommendationObservation {
        score: result.score.filter(|score| score.is_finite()),
        features: result
            .features
            .as_ref()
            .filter(|features| features.iter().all(|feature| feature.is_finite()))
            .cloned(),
    }
}

pub fn pick_unique_ids(
    pool: &[RecommendResult],
    taken: &HashSet<String>,
    limit: usize,
) -> Vec<String> {
    let mut out = Vec::with_capacity(limit);
    for it in pool {
        if out.len() >= limit {
            break;
        }
        let id = recommend_id_str(&it.id);
        if id.is_empty() || taken.contains(&id) {
            continue;
        }
        out.push(id);
    }
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn ids(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    fn neighbour(track_id: &str) -> ClusterNeighbor {
        ClusterNeighbor {
            track_id: track_id.to_owned(),
            artist_id: Uuid::nil(),
            artist_name: "Someone".to_owned(),
            avatar_url: None,
        }
    }

    #[test]
    fn a_track_already_on_a_shelf_is_not_put_on_a_second_one() {
        let mut builder = ClusterBuilder::new();
        builder.push("first", ids(&["a", "b"]));
        builder.push("second", ids(&["b", "c"]));
        let response = builder.finish();

        assert_eq!(
            response
                .clusters
                .iter()
                .map(|cluster| (cluster.id, cluster.track_ids.clone()))
                .collect::<Vec<_>>(),
            vec![("first", ids(&["a", "b"])), ("second", ids(&["c"]))],
            "the same track on two shelves of one page is a track the listener sees twice"
        );
    }

    #[test]
    fn a_shelf_left_with_nothing_of_its_own_does_not_appear_at_all() {
        let mut builder = ClusterBuilder::new();
        builder.push("first", ids(&["a"]));
        builder.push("second", ids(&["a"]));

        assert_eq!(
            builder
                .finish()
                .clusters
                .iter()
                .map(|cluster| cluster.id)
                .collect::<Vec<_>>(),
            vec!["first"],
            "an empty shelf must be absent, not present and empty"
        );
    }

    #[test]
    fn one_shelf_cannot_show_the_same_track_twice_within_itself() {
        let mut builder = ClusterBuilder::new();
        builder.push("collab", ids(&["a", "b", "a"]));
        builder.push_with_neighbors(
            "featured",
            vec![neighbour("c"), neighbour("d"), neighbour("c")],
        );
        let response = builder.finish();

        assert_eq!(
            response
                .clusters
                .iter()
                .map(|cluster| cluster.track_ids.clone())
                .collect::<Vec<_>>(),
            vec![ids(&["a", "b"]), ids(&["c", "d"])],
            "a track two artists share arrives twice from the query that ranks per artist, and \
             the listener would see one song filling two slots of the same row"
        );
        assert_eq!(
            response.clusters[1]
                .neighbors
                .as_ref()
                .map(|neighbors| neighbors.len()),
            Some(2),
            "the neighbour cards must be deduplicated with the tracks they belong to"
        );
    }

    #[test]
    fn a_shelf_that_owns_what_was_held_back_still_gets_to_show_it() {
        let mut builder = ClusterBuilder::new();
        builder.reserve(ids(&["own", "also_own"]));
        builder.push("borrowed", ids(&["own", "stranger"]));
        builder.push_reserved("mine", ids(&["own", "also_own"]));
        let response = builder.finish();

        assert_eq!(
            response
                .clusters
                .iter()
                .map(|cluster| (cluster.id, cluster.track_ids.clone()))
                .collect::<Vec<_>>(),
            vec![
                ("borrowed", ids(&["stranger"])),
                ("mine", ids(&["own", "also_own"])),
            ],
            "held back means other shelves may not take it, not that its own shelf loses it"
        );
    }

    #[test]
    fn even_its_own_shelf_cannot_show_what_is_already_on_the_page() {
        let mut builder = ClusterBuilder::new();
        builder.push("first", ids(&["a"]));
        builder.push_reserved("second", ids(&["a", "b"]));

        assert_eq!(
            builder
                .finish()
                .clusters
                .iter()
                .map(|cluster| cluster.track_ids.clone())
                .collect::<Vec<_>>(),
            vec![ids(&["a"]), ids(&["b"])]
        );
    }

    #[test]
    fn a_reserved_track_never_reaches_any_shelf() {
        let mut builder = ClusterBuilder::new();
        builder.reserve(ids(&["disliked"]));
        builder.push("first", ids(&["disliked", "fine"]));
        builder.push_with_neighbors("second", vec![neighbour("disliked"), neighbour("other")]);
        let response = builder.finish();

        assert_eq!(
            response
                .clusters
                .iter()
                .map(|cluster| cluster.track_ids.clone())
                .collect::<Vec<_>>(),
            vec![ids(&["fine"]), ids(&["other"])]
        );
        assert!(
            response.clusters[1]
                .neighbors
                .as_ref()
                .is_some_and(|neighbors| neighbors.len() == 1),
            "the neighbour card must go with the track it belongs to"
        );
    }

    #[test]
    fn observations_do_not_change_cluster_response_json() {
        let mut builder = ClusterBuilder::new();
        let results = [RecommendResult {
            id: json!("track"),
            score: Some(0.75),
            payload: None,
            artist: None,
            genre: None,
            playback_count: None,
            features: Some(vec![0.25]),
        }];
        builder.push_observed("wave", vec!["track".to_owned()], &results);

        assert_eq!(
            serde_json::to_value(builder.finish()).ok(),
            Some(json!({
                "clusters": [{
                    "id": "wave",
                    "track_ids": ["track"]
                }]
            }))
        );
    }

    #[test]
    fn observations_belong_only_to_selected_tracks() {
        let results = [
            RecommendResult {
                id: json!("selected"),
                score: Some(0.75),
                payload: None,
                artist: None,
                genre: None,
                playback_count: None,
                features: None,
            },
            RecommendResult {
                id: json!("filtered"),
                score: Some(0.25),
                payload: None,
                artist: None,
                genre: None,
                playback_count: None,
                features: None,
            },
        ];
        let mut builder = ClusterBuilder::new();
        builder.push_observed("wave", vec!["selected".to_owned()], &results);
        let response = builder.finish();

        assert_eq!(
            response.observation("selected").and_then(|item| item.score),
            Some(0.75)
        );
        assert!(response.observation("filtered").is_none());
    }
}
