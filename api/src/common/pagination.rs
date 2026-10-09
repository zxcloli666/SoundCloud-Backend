use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct PaginationQuery {
    #[serde(default, deserialize_with = "parse_opt_int")]
    pub page: Option<i64>,
    #[serde(default, deserialize_with = "parse_opt_int")]
    pub limit: Option<i64>,
}

const MAX_OFFSET: i64 = 50_000;

pub fn last_page(limit: i64) -> i64 {
    (MAX_OFFSET / limit.max(1) - 1).max(0)
}

impl PaginationQuery {
    pub fn page(&self) -> i64 {
        self.page.unwrap_or(0).clamp(0, last_page(self.limit()))
    }

    pub fn limit(&self) -> i64 {
        self.limit.unwrap_or(30).clamp(1, 200)
    }

    pub fn resolved(&self) -> (i64, i64) {
        (self.page(), self.limit())
    }
}

fn parse_opt_int<'de, D>(d: D) -> Result<Option<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let raw: Option<String> = Option::deserialize(d)?;
    match raw {
        None => Ok(None),
        Some(s) if s.is_empty() => Ok(None),
        Some(s) => s.parse::<i64>().map(Some).map_err(D::Error::custom),
    }
}

#[cfg(test)]
mod tests {
    use super::PaginationQuery;

    fn query(page: i64, limit: i64) -> PaginationQuery {
        PaginationQuery {
            page: Some(page),
            limit: Some(limit),
        }
    }

    #[test]
    fn a_page_is_bounded_by_its_offset_not_by_its_index() {
        assert_eq!(query(1_000, 30).page(), 1_000);
        assert_eq!(query(1_000, 200).page(), 249);
        assert_eq!(query(-3, 30).page(), 0);
    }
}
