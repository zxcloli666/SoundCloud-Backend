use super::Room;

impl Room {
    pub fn next_host(&self, leaving: &str, candidates: &[String]) -> Option<String> {
        self.members
            .iter()
            .filter(|m| m.user_id != leaving && candidates.contains(&m.user_id))
            .min_by(|a, b| {
                a.joined_at
                    .cmp(&b.joined_at)
                    .then_with(|| a.user_id.cmp(&b.user_id))
            })
            .map(|m| m.user_id.clone())
    }

    pub fn hand_off(&mut self, leaving: &str, candidates: &[String]) -> Option<String> {
        let next = self.next_host(leaving, candidates)?;
        self.remove(leaving);
        self.host_id = next.clone();
        self.moderators.retain(|id| id != &next);
        self.muted.retain(|m| m.user_id != next);
        Some(next)
    }
}
