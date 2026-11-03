INSERT INTO user_profiles (soundcloud_user_id, profile_json, synced_at, sc_observation)
VALUES ($1, $2, clock_timestamp(), $3)
ON CONFLICT (soundcloud_user_id) DO UPDATE
SET profile_json = EXCLUDED.profile_json,
    synced_at = EXCLUDED.synced_at,
    sc_observation = EXCLUDED.sc_observation
WHERE EXCLUDED.sc_observation > user_profiles.sc_observation
