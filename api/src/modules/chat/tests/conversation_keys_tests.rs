use super::*;

#[test]
fn a_dm_key_is_the_same_from_both_sides() {
    assert_eq!(dm_key("101", "202"), dm_key("202", "101"));
    assert_eq!(dm_key("101", "202"), "101/202");
}

#[test]
fn the_peer_comes_back_out_of_a_dm_key() {
    let key = dm_key("202", "101");
    assert_eq!(peer_of(&key, "101").as_deref(), Some("202"));
    assert_eq!(peer_of(&key, "202").as_deref(), Some("101"));
    assert_eq!(peer_of(&key, "303"), None);
}

#[test]
fn synthetic_ids_with_a_colon_still_split() {
    let key = dm_key("lt:2", "lt:10");
    assert_eq!(peer_of(&key, "lt:2").as_deref(), Some("lt:10"));
}

#[test]
fn a_saved_key_never_names_a_peer() {
    assert_eq!(saved_key("101"), "self/101");
    assert_eq!(peer_of(&saved_key("101"), "101"), None);
    assert_ne!(saved_key("101"), dm_key("101", "101"));
}

#[test]
fn user_ids_are_numeric_and_synthetic_ones_only_in_a_load_test() {
    assert!(valid_user_id("101", false));
    assert!(!valid_user_id("", false));
    assert!(!valid_user_id("10a", false));
    assert!(!valid_user_id("1/2", false));
    assert!(!valid_user_id(&"9".repeat(21), false));
    assert!(!valid_user_id("lt:5", false));
    assert!(valid_user_id("lt:5", true));
    assert!(!valid_user_id("lt:", true));
}
