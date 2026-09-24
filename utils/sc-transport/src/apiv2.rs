use serde_json::Value;
use wreq::header::HeaderMap;

use crate::ScClient;
use crate::error::{ScError, ScResult};
use crate::mapping::{
    self, PublicCollection, SearchType, collect_playlist_track_ids, index_tracks_by_id,
    reassemble_playlist_tracks,
};
use crate::pagination::{Page, parse_list_page};

const SC_API_V2: &str = "https://api-v2.soundcloud.com";
const HYDRATE_CHUNK: usize = 50;

pub struct Apiv2Proxy {
    sc: ScClient,
}

impl Apiv2Proxy {
    pub fn new(sc: ScClient) -> Self {
        Self { sc }
    }

    pub async fn resolve(&self, url: &str) -> ScResult<Value> {
        self.get_with_retry(|cid| {
            let q = url::form_urlencoded::Serializer::new(String::new())
                .append_pair("url", url)
                .append_pair("client_id", cid)
                .finish();
            format!("{SC_API_V2}/resolve?{q}")
        })
        .await
    }

    pub async fn track(&self, sc_track_id: &str) -> ScResult<Value> {
        self.get_with_retry(|cid| format!("{SC_API_V2}/tracks/{sc_track_id}?client_id={cid}"))
            .await
    }

    pub async fn user(&self, user_id: &str) -> ScResult<Value> {
        self.get_with_retry(|cid| format!("{SC_API_V2}/users/{user_id}?client_id={cid}"))
            .await
    }

    pub async fn playlist(&self, playlist_id: &str, hydrate: bool) -> ScResult<Value> {
        let mut playlist = self
            .get_with_retry(|cid| format!("{SC_API_V2}/playlists/{playlist_id}?client_id={cid}"))
            .await?;
        if !hydrate {
            mapping::normalize_v2_to_v1(&mut playlist);
            return Ok(playlist);
        }
        let (ids, embedded) = collect_playlist_track_ids(&playlist);
        let missing: Vec<&String> = ids
            .iter()
            .filter(|id| !embedded.contains_key(*id))
            .collect();

        let mut hydrated = std::collections::HashMap::new();
        for chunk in missing.chunks(HYDRATE_CHUNK) {
            let id_list = chunk
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(",");
            let arr = self
                .get_with_retry(|cid| format!("{SC_API_V2}/tracks?ids={id_list}&client_id={cid}"))
                .await?;
            if let Some(items) = arr.as_array() {
                hydrated.extend(index_tracks_by_id(items));
            }
        }

        let tracks = reassemble_playlist_tracks(&ids, &embedded, &hydrated);
        mapping::normalize_v2_to_v1(&mut playlist);
        if let Some(obj) = playlist.as_object_mut() {
            obj.insert("tracks".to_string(), Value::Array(tracks));
        }
        Ok(playlist)
    }

    pub async fn collection_page(
        &self,
        coll: PublicCollection,
        user_id: &str,
        cursor: Option<&str>,
        limit: i64,
    ) -> ScResult<Page> {
        let seg = coll.path_segment();
        let page = self
            .get_with_retry(|cid| match cursor {
                Some(c) => with_client_id(c, cid),
                None => format!(
                    "{SC_API_V2}/users/{user_id}/{seg}?client_id={cid}&limit={limit}&linked_partitioning=true"
                ),
            })
            .await?;
        parse_list_page(&page)
    }

    pub async fn search_page(
        &self,
        ty: SearchType,
        q: &str,
        cursor: Option<&str>,
        limit: i64,
    ) -> ScResult<Page> {
        let seg = ty.as_str();
        let page = self
            .get_with_retry(|cid| match cursor {
                Some(c) => with_client_id(c, cid),
                None => {
                    let qq = url::form_urlencoded::byte_serialize(q.as_bytes()).collect::<String>();
                    format!(
                        "{SC_API_V2}/search/{seg}?client_id={cid}&q={qq}&limit={limit}&linked_partitioning=true"
                    )
                }
            })
            .await?;
        parse_list_page(&page)
    }

    pub async fn get_list(&self, url: &str) -> ScResult<Page> {
        let page = self.get_with_retry(|cid| with_client_id(url, cid)).await?;
        parse_list_page(&page)
    }

    pub async fn get_value(&self, url: &str) -> ScResult<Value> {
        self.get_with_retry(|cid| with_client_id(url, cid)).await
    }

    async fn get_with_retry<F>(&self, build: F) -> ScResult<Value>
    where
        F: Fn(&str) -> String,
    {
        let cid = self.client_id().await?;
        match self.get_json(&build(&cid)).await {
            Ok(value) => Ok(value),
            Err(error) if rejects_our_client_id(&error) => {
                let cid = self.rotate_client_id(&cid).await?;
                self.get_json(&build(&cid)).await
            }
            Err(error) => Err(error),
        }
    }

    async fn get_json(&self, target_url: &str) -> ScResult<Value> {
        let bytes = self
            .sc
            .anon_get_via_relay_proxy(target_url, HeaderMap::new())
            .await?;
        crate::client::decode_json(&bytes)
    }

    async fn client_id(&self) -> ScResult<String> {
        self.sc.anon_client_id().await.ok_or_else(missing_client_id)
    }

    async fn rotate_client_id(&self, stale: &str) -> ScResult<String> {
        self.sc
            .rotate_anon_client_id(Some(stale))
            .await
            .ok_or_else(missing_client_id)
    }
}

fn missing_client_id() -> ScError {
    ScError::Unreachable("soundcloud.com did not hand out a client_id".to_owned())
}

fn rejects_our_client_id(error: &ScError) -> bool {
    matches!(error, ScError::Api { status, .. } if *status == 401 || *status == 403)
}

fn with_client_id(u: &str, cid: &str) -> String {
    if u.contains('?') {
        format!("{u}&client_id={cid}")
    } else {
        format!("{u}?client_id={cid}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_client_id_appends_correctly() {
        assert_eq!(
            with_client_id("https://x/y", "C"),
            "https://x/y?client_id=C"
        );
        assert_eq!(
            with_client_id("https://x/y?offset=z", "C"),
            "https://x/y?offset=z&client_id=C"
        );
    }

    #[test]
    fn parse_page_extracts_fields() {
        let v = serde_json::json!({
            "collection": [{"id": 1}, {"id": 2}],
            "next_href": "https://api-v2.soundcloud.com/x?offset=2",
            "total_results": 99
        });
        let p = parse_list_page(&v).unwrap();
        assert_eq!(p.items.len(), 2);
        assert_eq!(
            p.next_href.as_deref(),
            Some("https://api-v2.soundcloud.com/x?offset=2")
        );
    }

    #[test]
    fn parse_page_empty_next_href_is_none() {
        let v = serde_json::json!({"collection": [], "next_href": ""});
        let p = parse_list_page(&v).unwrap();
        assert!(p.items.is_empty());
        assert!(p.next_href.is_none());
    }
}
