use wreq::header::HeaderMap;

pub const PLAN_HEADER: &str = "sc-mob-userplan";

const UNKNOWN_KEPT_CHARS: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UserPlan {
    Free,
    Go,
    GoPlus,
    Student,
    Undefined,
    Unknown(String),
}

impl UserPlan {
    pub fn parse(value: &str) -> Self {
        let value = value.trim();
        match value.to_ascii_lowercase().as_str() {
            "none" => Self::Free,
            "mid_tier" => Self::Go,
            "high_tier" => Self::GoPlus,
            "student_tier" => Self::Student,
            "undefined" => Self::Undefined,
            _ => Self::Unknown(value.chars().take(UNKNOWN_KEPT_CHARS).collect()),
        }
    }

    pub fn from_headers(headers: &HeaderMap) -> Option<Self> {
        let value = headers.get(PLAN_HEADER)?;
        Some(Self::parse(&String::from_utf8_lossy(value.as_bytes())))
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Free => "none",
            Self::Go => "mid_tier",
            Self::GoPlus => "high_tier",
            Self::Student => "student_tier",
            Self::Undefined => "undefined",
            Self::Unknown(value) => value,
        }
    }
}

#[cfg(test)]
mod tests {
    use wreq::header::HeaderValue;

    use super::*;

    #[test]
    fn every_documented_plan_is_recognised() {
        for (header, plan) in [
            ("none", UserPlan::Free),
            ("mid_tier", UserPlan::Go),
            ("high_tier", UserPlan::GoPlus),
            ("student_tier", UserPlan::Student),
            ("undefined", UserPlan::Undefined),
            (" HIGH_TIER ", UserPlan::GoPlus),
        ] {
            assert_eq!(UserPlan::parse(header), plan, "{header}");
        }
    }

    #[test]
    fn an_unknown_plan_is_kept_not_refused() {
        assert_eq!(
            UserPlan::parse("ultra_tier"),
            UserPlan::Unknown("ultra_tier".to_owned())
        );
        assert_eq!(UserPlan::parse("ultra_tier").as_str(), "ultra_tier");
        assert_eq!(UserPlan::parse(""), UserPlan::Unknown(String::new()));
    }

    #[test]
    fn the_plan_is_read_from_the_response_header() {
        let mut headers = HeaderMap::new();
        assert_eq!(UserPlan::from_headers(&headers), None);

        headers.insert("SC-Mob-UserPlan", HeaderValue::from_static("mid_tier"));
        assert_eq!(UserPlan::from_headers(&headers), Some(UserPlan::Go));

        headers.insert(
            PLAN_HEADER,
            HeaderValue::from_bytes(b"tier\xff").unwrap_or(HeaderValue::from_static("x")),
        );
        assert!(matches!(
            UserPlan::from_headers(&headers),
            Some(UserPlan::Unknown(_))
        ));
    }
}
