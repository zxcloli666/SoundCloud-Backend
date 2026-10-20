use crate::modules::chat::synthetic::USER_PREFIX;

const SEPARATOR: char = '/';
const SAVED: &str = "self";
const ID_DIGITS_MAX: usize = 20;

pub fn dm_key(user: &str, other: &str) -> String {
    let (low, high) = if user <= other {
        (user, other)
    } else {
        (other, user)
    };
    format!("{low}{SEPARATOR}{high}")
}

pub fn saved_key(user: &str) -> String {
    format!("{SAVED}{SEPARATOR}{user}")
}

pub fn peer_of(key: &str, user: &str) -> Option<String> {
    let (low, high) = key.split_once(SEPARATOR)?;
    if low == SAVED {
        return None;
    }
    match (low == user, high == user) {
        (true, false) => Some(high.to_owned()),
        (false, true) => Some(low.to_owned()),
        _ => None,
    }
}

pub fn valid_user_id(id: &str, synthetic_allowed: bool) -> bool {
    let digits = match id.strip_prefix(USER_PREFIX) {
        Some(rest) if synthetic_allowed => rest,
        Some(_) => return false,
        None => id,
    };
    !digits.is_empty()
        && digits.len() <= ID_DIGITS_MAX
        && digits.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
#[path = "../tests/conversation_keys_tests.rs"]
mod tests;
