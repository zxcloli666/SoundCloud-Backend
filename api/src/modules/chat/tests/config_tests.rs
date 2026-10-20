use super::{Access, access, beta_users};

#[test]
fn anything_but_all_keeps_chat_in_beta() {
    assert_eq!(access("all"), Access::All);
    assert_eq!(access(" all "), Access::All);
    assert_eq!(access("beta"), Access::Beta);
    assert_eq!(access("ALL"), Access::Beta);
    assert_eq!(access(""), Access::Beta);
}

#[test]
fn beta_users_are_a_trimmed_comma_list() {
    let users = beta_users(" 101, 202,,303 ");
    assert_eq!(users.len(), 3);
    assert!(users.contains("101") && users.contains("202") && users.contains("303"));
    assert!(beta_users("").is_empty());
}
