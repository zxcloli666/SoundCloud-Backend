use serde_json::Value;

use crate::{ScError, ScResult};

pub struct Page {
    pub items: Vec<Value>,
    pub next_href: Option<String>,
}

pub fn parse_list_page(page: &Value) -> ScResult<Page> {
    let invalid = || ScError::invalid("invalid linked collection response");
    let object = page.as_object().ok_or_else(invalid)?;
    if object.contains_key("errors") || object.get("ok") == Some(&Value::Bool(false)) {
        return Err(invalid());
    }
    let items = object
        .get("collection")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    if items.len() > 200 || items.iter().any(|item| !item.is_object()) {
        return Err(invalid());
    }
    let next_href = match object.get("next_href") {
        None | Some(Value::Null) => None,
        Some(Value::String(cursor)) if cursor.is_empty() => None,
        Some(Value::String(cursor)) => {
            parse_list_cursor(cursor)?;
            Some(cursor.clone())
        }
        _ => return Err(invalid()),
    };
    Ok(Page {
        items: items.clone(),
        next_href,
    })
}

pub fn parse_list_cursor(cursor: &str) -> ScResult<url::Url> {
    let invalid = || ScError::invalid("invalid linked collection cursor");
    if cursor.len() > 4096 {
        return Err(invalid());
    }
    let url = url::Url::parse(cursor).map_err(|_| invalid())?;
    if url.scheme() != "https"
        || !matches!(
            url.host_str(),
            Some("api.soundcloud.com" | "api-v2.soundcloud.com")
        )
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn malformed_envelopes_are_not_empty_collections() {
        for value in [
            Value::Null,
            json!([]),
            json!({"errors": []}),
            json!({"collection": null}),
            json!({"collection": [null]}),
            json!({"collection": [], "next_href": 1}),
            json!({"collection": [], "ok": false}),
        ] {
            assert!(parse_list_page(&value).is_err());
        }
    }

    #[test]
    fn empty_pages_can_still_have_a_valid_continuation() -> ScResult<()> {
        let page = parse_list_page(
            &json!({"collection": [], "next_href": "https://api.soundcloud.com/tracks/42/comments?cursor=next"}),
        )?;
        assert!(page.items.is_empty());
        assert!(page.next_href.is_some());
        assert!(
            parse_list_page(&json!({"collection": [], "next_href": null}))?
                .next_href
                .is_none()
        );
        Ok(())
    }

    #[test]
    fn cursor_host_cannot_be_forged_in_a_query_or_authority() {
        for cursor in [
            "https://example.test/?api.soundcloud.com",
            "https://api.soundcloud.com.example.test/x",
            "https://api.soundcloud.com@example.test/x",
            "http://api.soundcloud.com/x",
            "https://api.soundcloud.com:444/x",
            "https://api-v2.soundcloud.com/x#secret",
        ] {
            assert!(parse_list_cursor(cursor).is_err());
        }
        assert!(
            parse_list_cursor("https://api-v2.soundcloud.com/tracks/42/comments?cursor=x").is_ok()
        );
    }
}
