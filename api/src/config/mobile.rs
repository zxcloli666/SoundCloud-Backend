const EVERYONE: &str = "all";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum MobileLinkAllow {
    #[default]
    Nobody,
    Everyone,
    Accounts(Vec<String>),
}

impl MobileLinkAllow {
    pub fn parse(raw: &str) -> Self {
        let raw = raw.trim();
        if raw.is_empty() {
            return Self::Nobody;
        }
        if raw.eq_ignore_ascii_case(EVERYONE) {
            return Self::Everyone;
        }
        Self::Accounts(
            raw.split(',')
                .map(str::trim)
                .filter(|account| !account.is_empty())
                .map(str::to_owned)
                .collect(),
        )
    }

    pub fn admits(&self, soundcloud_user_id: &str) -> bool {
        match self {
            Self::Nobody => false,
            Self::Everyone => true,
            Self::Accounts(accounts) => {
                accounts.iter().any(|account| account == soundcloud_user_id)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_value_keeps_the_feature_off() {
        assert!(!MobileLinkAllow::parse("").admits("4242"));
        assert!(!MobileLinkAllow::parse(" , ").admits("4242"));
    }

    #[test]
    fn a_list_admits_only_its_accounts() {
        let allow = MobileLinkAllow::parse("4242, 77");

        assert!(allow.admits("4242"));
        assert!(allow.admits("77"));
        assert!(!allow.admits("424"));
    }

    #[test]
    fn all_admits_everyone() {
        assert!(MobileLinkAllow::parse("ALL").admits("1"));
    }
}
