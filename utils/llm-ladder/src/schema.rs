use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Schema {
    Object(Vec<(String, Schema)>),
    Array(Box<Schema>),
    String,
    OneOf(Vec<String>),
    Integer,
    Number,
    Boolean,
    Null,
    AnyOf(Vec<Schema>),
}

impl Schema {
    pub fn object<K: Into<String>>(properties: impl IntoIterator<Item = (K, Schema)>) -> Self {
        Self::Object(
            properties
                .into_iter()
                .map(|(name, schema)| (name.into(), schema))
                .collect(),
        )
    }

    pub fn array(items: Schema) -> Self {
        Self::Array(Box::new(items))
    }

    pub fn one_of<V: Into<String>>(values: impl IntoIterator<Item = V>) -> Self {
        Self::OneOf(values.into_iter().map(Into::into).collect())
    }
}

impl Serialize for Schema {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        match self {
            Self::Object(properties) => {
                map.serialize_entry("type", "object")?;
                map.serialize_entry("properties", &Properties(properties))?;
                map.serialize_entry("required", &Names(properties))?;
                map.serialize_entry("additionalProperties", &false)?;
            }
            Self::Array(items) => {
                map.serialize_entry("type", "array")?;
                map.serialize_entry("items", items)?;
            }
            Self::String => map.serialize_entry("type", "string")?,
            Self::OneOf(values) => {
                map.serialize_entry("type", "string")?;
                map.serialize_entry("enum", values)?;
            }
            Self::Integer => map.serialize_entry("type", "integer")?,
            Self::Number => map.serialize_entry("type", "number")?,
            Self::Boolean => map.serialize_entry("type", "boolean")?,
            Self::Null => map.serialize_entry("type", "null")?,
            Self::AnyOf(options) => map.serialize_entry("anyOf", options)?,
        }
        map.end()
    }
}

struct Properties<'a>(&'a [(String, Schema)]);

impl Serialize for Properties<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (name, schema) in self.0 {
            map.serialize_entry(name, schema)?;
        }
        map.end()
    }
}

struct Names<'a>(&'a [(String, Schema)]);

impl Serialize for Names<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut names = serializer.serialize_seq(Some(self.0.len()))?;
        for (name, _) in self.0 {
            names.serialize_element(name)?;
        }
        names.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer() -> Schema {
        Schema::object([
            (
                "actions",
                Schema::array(Schema::object([
                    ("name", Schema::one_of(["volume_set", "pause"])),
                    (
                        "args",
                        Schema::array(Schema::object([
                            ("name", Schema::String),
                            (
                                "value",
                                Schema::AnyOf(vec![
                                    Schema::String,
                                    Schema::Number,
                                    Schema::Boolean,
                                ]),
                            ),
                        ])),
                    ),
                ])),
            ),
            ("say", Schema::String),
        ])
    }

    #[test]
    fn properties_keep_the_order_they_were_given_in() {
        let text = serde_json::to_string(&answer()).unwrap();
        let name = text.find("\"name\":{\"type\":\"string\",\"enum\"").unwrap();
        let args = text.find("\"args\":{").unwrap();
        let actions = text.find("\"actions\":{").unwrap();
        let say = text.find("\"say\":{").unwrap();
        assert!(actions < name && name < args && args < say, "{text}");
    }

    #[test]
    fn every_object_is_closed_and_requires_all_of_its_properties() {
        let value = serde_json::to_value(answer()).unwrap();
        assert_eq!(value["additionalProperties"], false);
        assert_eq!(value["required"], serde_json::json!(["actions", "say"]));
        let action = &value["properties"]["actions"]["items"];
        assert_eq!(action["additionalProperties"], false);
        assert_eq!(action["required"], serde_json::json!(["name", "args"]));
        assert_eq!(
            action["properties"]["args"]["items"]["properties"]["value"]["anyOf"],
            serde_json::json!([{"type": "string"}, {"type": "number"}, {"type": "boolean"}])
        );
    }

    #[test]
    fn scalars_and_choices_serialize_as_plain_json_schema() {
        assert_eq!(
            serde_json::to_value(Schema::one_of(["a", "b"])).unwrap(),
            serde_json::json!({"type": "string", "enum": ["a", "b"]})
        );
        assert_eq!(
            serde_json::to_value(Schema::AnyOf(vec![Schema::Integer, Schema::Null])).unwrap(),
            serde_json::json!({"anyOf": [{"type": "integer"}, {"type": "null"}]})
        );
    }
}
