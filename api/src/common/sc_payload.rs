use serde_json::{Value, json};

pub fn validate_entity_identity(
    payload: &Value,
    entity: backend_contracts::CatalogEntity,
    id: &str,
) -> crate::error::AppResult<()> {
    let expected_urn = entity.urn(id);
    let actual_id = payload.get("id").map(|value| match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        _ => String::new(),
    });
    if actual_id.as_deref() != Some(id)
        || payload
            .get("urn")
            .is_some_and(|urn| urn.as_str() != Some(&expected_urn))
        || payload
            .get("kind")
            .is_some_and(|kind| kind.as_str() != Some(entity.as_str()))
    {
        return Err(crate::error::AppError::coded(
            axum::http::StatusCode::BAD_GATEWAY,
            "invalid_catalog_response",
            "SoundCloud returned a different entity",
        ));
    }
    Ok(())
}

pub fn parse_id_or_string(s: &str) -> Value {
    s.parse::<i64>()
        .map(|n| json!(n))
        .unwrap_or_else(|_| Value::String(s.to_string()))
}
