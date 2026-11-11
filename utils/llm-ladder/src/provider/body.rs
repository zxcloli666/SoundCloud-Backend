use serde::ser::{Serialize, SerializeMap, Serializer};
use serde_json::{Map, Value};

use crate::schema::Schema;

pub(crate) struct Body<'a> {
    pub value: Value,
    pub schema_at: &'static [&'static str],
    pub schema: &'a Schema,
}

pub(crate) fn merge(base: &mut Value, extra: &Map<String, Value>) {
    let Value::Object(base) = base else {
        return;
    };
    for (key, value) in extra {
        match (base.get_mut(key), value) {
            (Some(existing @ Value::Object(_)), Value::Object(nested)) => merge(existing, nested),
            _ => {
                base.insert(key.clone(), value.clone());
            }
        }
    }
}

impl Serialize for Body<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Node {
            value: &self.value,
            schema_at: self.schema_at,
            schema: self.schema,
        }
        .serialize(serializer)
    }
}

struct Node<'a> {
    value: &'a Value,
    schema_at: &'a [&'static str],
    schema: &'a Schema,
}

impl Serialize for Node<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let (Value::Object(fields), [next, deeper @ ..]) = (self.value, self.schema_at) else {
            return self.value.serialize(serializer);
        };
        let mut map = serializer.serialize_map(None)?;
        for (name, value) in fields {
            if name != next {
                map.serialize_entry(name, value)?;
            } else if !deeper.is_empty() {
                let nested = Node {
                    value,
                    schema_at: deeper,
                    schema: self.schema,
                };
                map.serialize_entry(name, &nested)?;
            }
        }
        if deeper.is_empty() {
            map.serialize_entry(next, self.schema)?;
        }
        map.end()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn schema() -> Schema {
        Schema::object([("zeta", Schema::String), ("alpha", Schema::Integer)])
    }

    #[test]
    fn the_schema_lands_at_its_path_with_its_own_property_order() {
        let body = Body {
            value: json!({"model": "example-model", "output_config": {"format": {"type": "json_schema"}}}),
            schema_at: &["output_config", "format", "schema"],
            schema: &schema(),
        };
        let text = serde_json::to_string(&body).unwrap();
        assert!(
            text.find("\"zeta\"").unwrap() < text.find("\"alpha\"").unwrap(),
            "{text}"
        );
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["model"], "example-model");
        assert_eq!(value["output_config"]["format"]["type"], "json_schema");
        assert_eq!(
            value["output_config"]["format"]["schema"]["required"],
            json!(["zeta", "alpha"])
        );
    }

    #[test]
    fn a_config_body_cannot_replace_the_schema_and_a_cut_path_drops_it() {
        let stale = Body {
            value: json!({"response_format": {"json_schema": {"schema": "stale", "strict": true}}}),
            schema_at: &["response_format", "json_schema", "schema"],
            schema: &schema(),
        };
        let value = serde_json::to_value(&stale).unwrap();
        assert_eq!(
            value["response_format"]["json_schema"]["schema"]["type"],
            "object"
        );
        assert_eq!(value["response_format"]["json_schema"]["strict"], true);
        let cut = Body {
            value: json!({"response_format": "text"}),
            schema_at: &["response_format", "json_schema", "schema"],
            schema: &schema(),
        };
        assert_eq!(
            serde_json::to_value(&cut).unwrap(),
            json!({"response_format": "text"})
        );
    }

    #[test]
    fn config_knobs_merge_into_nested_objects_and_win_over_scalars() {
        let mut base =
            json!({"max_tokens": 100, "output_config": {"format": {"type": "json_schema"}}});
        let extra = json!({"max_tokens": 200, "thinking": {"type": "disabled"}, "output_config": {"effort": "low"}});
        merge(&mut base, extra.as_object().unwrap());
        assert_eq!(
            base,
            json!({
                "max_tokens": 200,
                "thinking": {"type": "disabled"},
                "output_config": {"effort": "low", "format": {"type": "json_schema"}},
            })
        );
    }
}
