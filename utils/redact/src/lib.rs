use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub const MASK: &str = "[redacted]";

#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct Secret<T>(T);

impl<T> Secret<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }

    pub fn expose(&self) -> &T {
        &self.0
    }

    pub fn into_inner(self) -> T {
        self.0
    }
}

impl Secret<String> {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<T> fmt::Debug for Secret<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(MASK)
    }
}

impl<T> fmt::Display for Secret<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(MASK)
    }
}

impl<T> From<T> for Secret<T> {
    fn from(value: T) -> Self {
        Self(value)
    }
}

impl<T: Serialize> Serialize for Secret<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Secret<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        T::deserialize(deserializer).map(Self)
    }
}

pub fn url(connection_string: &str) -> String {
    let Some((scheme, rest)) = connection_string.split_once("://") else {
        return connection_string.to_owned();
    };
    let Some((userinfo, tail)) = rest.split_once('@') else {
        return connection_string.to_owned();
    };
    let user = userinfo.split_once(':').map_or(userinfo, |(user, _)| user);
    format!("{scheme}://{user}:***@{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_prints_as_nothing_however_it_is_asked() {
        let token = Secret::new("s3cr3t-admin-token".to_owned());

        assert_eq!(format!("{token:?}"), MASK);
        assert_eq!(format!("{token}"), MASK);
        assert_eq!(format!("{token:#?}"), MASK);
        assert!(
            !format!("{:?}", vec![token.clone()]).contains("s3cr3t"),
            "a derived Debug on any struct holding one must stay clean too"
        );
    }

    #[test]
    fn a_secret_still_hands_over_the_value_when_it_is_actually_used() {
        let token = Secret::new("s3cr3t".to_owned());

        assert_eq!(token.expose(), "s3cr3t");
        assert_eq!(token.into_inner(), "s3cr3t");
    }

    #[test]
    fn a_secret_travels_through_json_unchanged() {
        let parsed: Secret<String> =
            serde_json::from_str("\"from-the-database\"").expect("a plain string deserializes");
        assert_eq!(parsed.expose(), "from-the-database");
        assert_eq!(
            serde_json::to_string(&parsed).expect("serializes"),
            "\"from-the-database\"",
            "the wrapper guards logs, not the answer an admin explicitly asked for"
        );
    }

    #[test]
    fn a_password_inside_a_connection_string_is_masked_and_the_rest_is_kept() {
        assert_eq!(
            url("postgres://soundcloud:hunter2@db.internal:5432/scd"),
            "postgres://soundcloud:***@db.internal:5432/scd"
        );
        assert_eq!(
            url("redis://:hunter2@cache.internal:6379"),
            "redis://:***@cache.internal:6379"
        );
    }

    #[test]
    fn a_connection_string_without_a_password_is_left_readable() {
        for plain in [
            "redis://localhost:6379",
            "nats://nats:4222",
            "not a url at all",
        ] {
            assert_eq!(url(plain), plain);
        }
    }
}
