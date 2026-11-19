INSERT INTO mobile_link_requests (
    id,
    state,
    code_verifier,
    oauth_app_id,
    session_id,
    expected_soundcloud_user_id,
    device_id,
    expires_at
)
VALUES ($1, $2, $3, $4, $5, $6, $7, now() + $8::int * interval '1 second')
