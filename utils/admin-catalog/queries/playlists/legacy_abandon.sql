UPDATE playlist_legacy_membership_intents
SET prior_classification = classification,
    classification = 'abandoned',
    resolved_at = clock_timestamp()
WHERE archive_id = $1
  AND resolved_at IS NULL
RETURNING playlist_urn AS "playlist_urn!"
