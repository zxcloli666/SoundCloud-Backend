use std::time::Duration;

use chrono::{TimeZone, Utc};

use super::{
    Access, MODEL_TIMEOUT_DEFAULT, ModelChoice, access, beta_users, loadtest_until, models,
};

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

fn choice(name: &str, secs: u64) -> ModelChoice {
    ModelChoice {
        name: name.to_owned(),
        timeout: Duration::from_secs(secs),
    }
}

#[test]
fn models_are_an_ordered_list_with_their_own_timeouts() {
    assert_eq!(
        models("fast,google", "10,30"),
        vec![choice("fast", 10), choice("google", 30)]
    );
    assert_eq!(
        models(" fast , google ", " 4 "),
        vec![choice("fast", 4), choice("google", 30)]
    );
    assert_eq!(models("fast", ""), vec![choice("fast", 30)]);
    assert_eq!(models("", "10"), Vec::new());
}

#[test]
fn a_missing_or_bad_timeout_falls_back_to_thirty_seconds() {
    assert_eq!(MODEL_TIMEOUT_DEFAULT, Duration::from_secs(30));
    assert_eq!(
        models("a,b,c", "x,0,7"),
        vec![choice("a", 30), choice("b", 30), choice("c", 7)]
    );
    assert_eq!(
        models("a,,c", "1,2,3"),
        vec![choice("a", 1), choice("c", 3)]
    );
}

#[test]
fn the_load_test_deadline_takes_rfc_3339_or_epoch_seconds() {
    let at = Utc.timestamp_opt(1_793_400_000, 0).single();
    assert_eq!(
        loadtest_until("2026-10-31T00:00:00Z"),
        Utc.with_ymd_and_hms(2026, 10, 31, 0, 0, 0).single()
    );
    assert_eq!(loadtest_until("1793400000"), at);
    assert_eq!(loadtest_until(" 1793400000 "), at);
    assert_eq!(
        loadtest_until("2026-11-04T01:00:00+03:00"),
        loadtest_until("2026-11-03T22:00:00Z")
    );
    assert_eq!(loadtest_until(""), None);
    assert_eq!(loadtest_until("  "), None);
}

#[test]
#[should_panic(expected = "CHAT_LOADTEST_UNTIL")]
fn a_garbled_load_test_deadline_stops_the_boot() {
    loadtest_until("tomorrow");
}
