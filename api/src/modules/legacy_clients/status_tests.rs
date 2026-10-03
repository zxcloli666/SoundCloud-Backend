use serde_json::json;

use super::SIGNED_OUT;

#[test]
fn a_signed_out_status_keeps_the_fields_old_desktops_read() {
    assert_eq!(
        serde_json::to_value(SIGNED_OUT).unwrap(),
        json!({
            "authenticated": false,
            "tokenState": "expired",
            "pendingSyncCount": 0,
            "failedSyncCount": 0
        })
    );
}
