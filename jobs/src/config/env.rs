use std::fmt::{Display, Formatter};
use std::str::FromStr;

#[derive(Debug)]
pub enum ConfigError {
    Invalid { key: String, reason: String },
    Missing(String),
}

impl Display for ConfigError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid { key, reason } => write!(formatter, "invalid {key}: {reason}"),
            Self::Missing(key) => write!(formatter, "missing required environment variable {key}"),
        }
    }
}

impl std::error::Error for ConfigError {}

pub fn optional(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|value| !value.is_empty())
}

pub fn required(key: &str) -> Result<String, ConfigError> {
    optional(key).ok_or_else(|| ConfigError::Missing(key.to_owned()))
}

pub fn parse<T>(key: &str, default: &str) -> Result<T, ConfigError>
where
    T: FromStr,
    T::Err: Display,
{
    let value = match optional(key) {
        Some(value) => value,
        None => default.to_owned(),
    };
    value.parse::<T>().map_err(|error| ConfigError::Invalid {
        key: key.to_owned(),
        reason: error.to_string(),
    })
}

pub fn positive_u64(key: &str, default: u64) -> Result<u64, ConfigError> {
    let value: u64 = parse(key, &default.to_string())?;
    if value == 0 {
        return Err(ConfigError::Invalid {
            key: key.to_owned(),
            reason: "must be greater than zero".to_owned(),
        });
    }
    Ok(value)
}

pub fn non_zero_usize(key: &str, default: usize) -> Result<usize, ConfigError> {
    let value: usize = parse(key, &default.to_string())?;
    if value == 0 {
        return Err(ConfigError::Invalid {
            key: key.to_owned(),
            reason: "must be greater than zero".to_owned(),
        });
    }
    Ok(value)
}

pub fn boolean(key: &str, default: bool) -> Result<bool, ConfigError> {
    match optional(key).as_deref() {
        None => Ok(default),
        Some("true" | "1") => Ok(true),
        Some("false" | "0") => Ok(false),
        Some(_) => Err(ConfigError::Invalid {
            key: key.to_owned(),
            reason: "expected true, false, 1, or 0".to_owned(),
        }),
    }
}
