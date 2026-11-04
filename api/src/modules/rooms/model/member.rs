use super::{Member, Profile};

const MAX_NAME_CHARS: usize = 64;
const MAX_AVATAR_CHARS: usize = 512;

impl Member {
    pub fn new(user_id: &str, profile: &Profile, now: i64) -> Self {
        Self {
            user_id: user_id.to_owned(),
            name: clean_name(&profile.name, user_id),
            avatar_url: clean_avatar(profile.avatar_url.as_deref()),
            joined_at: now,
            ready_urn: None,
            chat_from_seq: 0,
        }
    }

    pub fn reading_from(mut self, seq: i64) -> Self {
        self.chat_from_seq = seq.max(0);
        self
    }
}

fn clean_name(name: &str, fallback: &str) -> String {
    let name: String = name
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME_CHARS)
        .collect();
    if name.is_empty() {
        fallback.to_owned()
    } else {
        name
    }
}

fn clean_avatar(url: Option<&str>) -> Option<String> {
    let url = url?.trim();
    (url.starts_with("https://") && url.len() <= MAX_AVATAR_CHARS).then(|| url.to_owned())
}
