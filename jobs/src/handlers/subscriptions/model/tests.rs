use super::*;

fn entry(user_urn: &str, exp_date: i64) -> Subscription {
    Subscription {
        user_urn: user_urn.to_owned(),
        exp_date,
    }
}

#[test]
fn snapshot_round_trips_through_json() -> Result<(), Box<dyn std::error::Error>> {
    let snapshot = SubscriptionSnapshot::from_entries(vec![entry("123", 456)], 1)?;
    let encoded = snapshot.encode(1_024)?;
    let decoded = SubscriptionSnapshot::decode(&encoded, 1)?;

    assert_eq!(decoded.len(), 1);
    Ok(())
}

#[test]
fn snapshot_rejects_duplicate_user_urns() {
    let error = SubscriptionSnapshot::from_entries(
        vec![
            entry("soundcloud:users:1", 10),
            entry("soundcloud:users:1", 20),
        ],
        2,
    );

    assert!(matches!(
        error,
        Err(SnapshotError::DuplicateUserUrn { index: 1 })
    ));
}

#[test]
fn snapshot_encoding_stops_at_byte_limit() {
    let snapshot = SubscriptionSnapshot::from_entries(vec![entry("123", 456)], 1);
    let result = snapshot.and_then(|snapshot| snapshot.encode(4));

    assert!(matches!(result, Err(SnapshotError::TooLarge { .. })));
}

#[test]
fn snapshot_rejects_more_entries_than_configured() {
    let result = SubscriptionSnapshot::from_entries(vec![entry("1", 10), entry("2", 20)], 1);

    assert!(matches!(
        result,
        Err(SnapshotError::TooManyEntries { max_entries: 1 })
    ));
}
