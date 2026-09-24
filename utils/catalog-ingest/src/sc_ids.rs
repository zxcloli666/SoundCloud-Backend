pub fn normalize_sc_track_id(input: &str) -> Option<String> {
    if input.is_empty() {
        return None;
    }
    let last = if input.contains(':') {
        input.rsplit(':').next().unwrap_or("")
    } else {
        input
    };
    if !last.is_empty() && last.bytes().all(|b| b.is_ascii_digit()) {
        Some(last.to_string())
    } else {
        None
    }
}

pub fn extract_sc_id(urn: &str) -> &str {
    urn.rsplit_once(':').map(|(_, id)| id).unwrap_or(urn)
}

pub fn user_urn(sc_user_id: &str) -> String {
    format!("soundcloud:users:{}", extract_sc_id(sc_user_id))
}

pub fn user_id_variants(sc_user_id: &str) -> Vec<String> {
    let trimmed = sc_user_id.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let bare = trimmed.rsplit(':').next().unwrap_or(trimmed);
    let mut out = vec![trimmed.to_string()];
    if bare != trimmed {
        out.push(bare.to_string());
    }
    if bare.bytes().all(|b| b.is_ascii_digit()) && !bare.is_empty() {
        let urn = format!("soundcloud:users:{bare}");
        if urn != trimmed {
            out.push(urn);
        }
    }
    out.dedup();
    out
}
