pub use entity_ref::{EntityKind, EntityRef};

pub fn normalize_sc_track_id(input: &str) -> Option<String> {
    entity_ref::sc_track_id(input)
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

pub(crate) fn payload_ref(kind: EntityKind, payload: &serde_json::Value) -> Option<EntityRef> {
    let parsed = EntityRef::parse_urn(payload.get("urn")?.as_str()?)?;
    (parsed.kind() == kind).then_some(parsed)
}

pub(crate) fn user_ref(user: &serde_json::Value) -> Option<EntityRef> {
    payload_ref(EntityKind::User, user).or_else(|| match user.get("id")? {
        serde_json::Value::Number(id) => EntityRef::new(EntityKind::User, id.as_u64()?),
        serde_json::Value::String(id) => EntityRef::user(id),
        _ => None,
    })
}
