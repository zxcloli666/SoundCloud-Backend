use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListPageResult<T> {
    pub collection: Vec<T>,
    pub page: i64,
    pub page_size: i64,
    pub has_more: bool,
}

pub fn build_list_cache_key(prefix: &str, params: &[(&str, String)]) -> String {
    let mut parts: Vec<String> = params
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| format!("{}:{k}:{}:{v}", k.len(), v.len()))
        .collect();
    if parts.is_empty() {
        return prefix.to_string();
    }
    parts.sort();
    let digest = Sha256::digest(parts.join("&").as_bytes());
    format!("{prefix}:{}", hex::encode(digest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_keys_cannot_be_confused_by_separators_inside_values() {
        assert_ne!(
            build_list_cache_key("likes", &[("a", "x&b=y".to_owned())]),
            build_list_cache_key("likes", &[("a", "x".to_owned()), ("b", "y".to_owned())])
        );
        assert_eq!(build_list_cache_key("likes", &[]), "likes");
        assert_eq!(
            build_list_cache_key("likes", &[("a", String::new())]),
            "likes"
        );
    }
}
