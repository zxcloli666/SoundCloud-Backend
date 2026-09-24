INSERT INTO user_web_profiles (sc_user_id, profiles, sc_observation)
VALUES ($1, $2, $3)
ON CONFLICT (sc_user_id) DO UPDATE
SET profiles = EXCLUDED.profiles,
    sc_observation = EXCLUDED.sc_observation,
    synced_at = now()
WHERE user_web_profiles.sc_observation < EXCLUDED.sc_observation
