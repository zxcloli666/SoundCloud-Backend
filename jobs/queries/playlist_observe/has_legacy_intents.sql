SELECT EXISTS (
    SELECT 1
    FROM playlist_legacy_membership_intents
    WHERE playlist_urn = $1
      AND classification NOT IN ('resolved', 'abandoned')
) AS "has_legacy_intents!"
