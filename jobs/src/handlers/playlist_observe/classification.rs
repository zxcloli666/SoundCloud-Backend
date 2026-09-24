use std::collections::HashSet;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MembershipRelation {
    Equal,
    RemoteSuperset,
    LocalSuperset,
    OrderOnly,
    Diverged,
}

impl MembershipRelation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Equal => "equal",
            Self::RemoteSuperset => "remote_superset",
            Self::LocalSuperset => "local_superset",
            Self::OrderOnly => "order_only",
            Self::Diverged => "membership_diverged",
        }
    }
}

pub fn membership_relation(local: &[String], remote: &[String]) -> MembershipRelation {
    if local == remote {
        return MembershipRelation::Equal;
    }
    let local_members = local.iter().collect::<HashSet<_>>();
    let remote_members = remote.iter().collect::<HashSet<_>>();
    if local_members == remote_members {
        return MembershipRelation::OrderOnly;
    }
    if local.len() < remote.len() && is_subsequence(local, remote) {
        return MembershipRelation::RemoteSuperset;
    }
    if remote.len() < local.len() && is_subsequence(remote, local) {
        return MembershipRelation::LocalSuperset;
    }
    MembershipRelation::Diverged
}

fn is_subsequence(left: &[String], right: &[String]) -> bool {
    let mut right = right.iter();
    left.iter()
        .all(|expected| right.by_ref().any(|candidate| candidate == expected))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn classifies_safe_remote_superset_only_when_order_is_preserved() {
        assert_eq!(
            membership_relation(&ids(&["1", "3"]), &ids(&["1", "2", "3"])),
            MembershipRelation::RemoteSuperset
        );
        assert_eq!(
            membership_relation(&ids(&["3", "1"]), &ids(&["1", "2", "3"])),
            MembershipRelation::Diverged
        );
    }

    #[test]
    fn separates_reordering_from_membership_changes() {
        assert_eq!(
            membership_relation(&ids(&["1", "2"]), &ids(&["2", "1"])),
            MembershipRelation::OrderOnly
        );
        assert_eq!(
            membership_relation(&ids(&["1", "4"]), &ids(&["1", "2", "3"])),
            MembershipRelation::Diverged
        );
    }
}
