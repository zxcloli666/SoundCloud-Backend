UPDATE playlist_legacy_membership_intents
SET local_snapshot_id = COALESCE(local_snapshot_id, $2),
    classification = $3
WHERE playlist_urn = $1
  AND classification NOT IN ('resolved', 'abandoned')
