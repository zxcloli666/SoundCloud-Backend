use super::ConfigError;
use super::env;

const NODES: [&str; 3] = ["core", "main", "star"];
const ROLES: [&str; 2] = ["core", "api"];
const MODES: [&str; 2] = ["legacy", "split"];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScdNode {
    pub node: String,
    pub role: String,
    pub mode: String,
}

impl ScdNode {
    pub fn from_env() -> Result<Option<Self>, ConfigError> {
        Self::parse(
            env::optional("SCD_NODE"),
            env::optional("SCD_ROLE"),
            env::optional("SCD_MODE"),
        )
    }

    pub fn split(&self) -> bool {
        self.mode == "split"
    }

    fn parse(
        node: Option<String>,
        role: Option<String>,
        mode: Option<String>,
    ) -> Result<Option<Self>, ConfigError> {
        if node.is_none() && role.is_none() && mode.is_none() {
            return Ok(None);
        }
        let node = one_of("SCD_NODE", node, &NODES)?;
        let role = one_of("SCD_ROLE", role, &ROLES)?;
        let mode = one_of("SCD_MODE", mode, &MODES)?;
        if (role == "core") != (node == "core") {
            return Err(ConfigError::Invalid {
                key: "SCD_ROLE".to_owned(),
                reason: "must be core on node core and api on main and star".to_owned(),
            });
        }
        Ok(Some(Self { node, role, mode }))
    }
}

fn one_of(key: &str, value: Option<String>, allowed: &[&str]) -> Result<String, ConfigError> {
    let value = value.ok_or_else(|| ConfigError::Missing(key.to_owned()))?;
    if allowed.contains(&value.as_str()) {
        return Ok(value);
    }
    Err(ConfigError::Invalid {
        key: key.to_owned(),
        reason: format!("must be one of {}", allowed.join(", ")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(
        node: Option<&str>,
        role: Option<&str>,
        mode: Option<&str>,
    ) -> Result<Option<ScdNode>, ConfigError> {
        ScdNode::parse(
            node.map(str::to_owned),
            role.map(str::to_owned),
            mode.map(str::to_owned),
        )
    }

    #[test]
    fn unset_means_unconfigured() {
        assert_eq!(parse(None, None, None).unwrap(), None);
    }

    #[test]
    fn all_three_are_required_together() {
        assert!(
            matches!(parse(Some("main"), None, None), Err(ConfigError::Missing(key)) if key == "SCD_ROLE")
        );
        assert!(
            matches!(parse(Some("main"), Some("api"), None), Err(ConfigError::Missing(key)) if key == "SCD_MODE")
        );
        assert!(
            matches!(parse(None, None, Some("split")), Err(ConfigError::Missing(key)) if key == "SCD_NODE")
        );
    }

    #[test]
    fn role_follows_the_node() {
        assert!(parse(Some("core"), Some("api"), Some("legacy")).is_err());
        assert!(parse(Some("star"), Some("core"), Some("legacy")).is_err());
        let node = parse(Some("star"), Some("api"), Some("split"))
            .unwrap()
            .unwrap();
        assert!(node.split());
        assert!(
            !parse(Some("core"), Some("core"), Some("legacy"))
                .unwrap()
                .unwrap()
                .split()
        );
    }

    #[test]
    fn unknown_values_are_rejected() {
        assert!(parse(Some("load"), Some("api"), Some("legacy")).is_err());
        assert!(parse(Some("main"), Some("api"), Some("hub")).is_err());
    }
}
