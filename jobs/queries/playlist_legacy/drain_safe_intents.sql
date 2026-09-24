WITH drained AS (
    SELECT archive_id,
           classification
    FROM playlist_legacy_membership_intents
    WHERE resolved_at IS NULL
      AND classification IN ('equal', 'remote_superset', 'order_only')
    ORDER BY archived_at, archive_id
    LIMIT $1
    FOR UPDATE SKIP LOCKED
)
UPDATE playlist_legacy_membership_intents AS intent
SET prior_classification = drained.classification,
    classification = CASE drained.classification
        WHEN 'order_only' THEN 'abandoned'
        ELSE 'resolved'
    END,
    resolved_at = clock_timestamp()
FROM drained
WHERE intent.archive_id = drained.archive_id
RETURNING intent.playlist_urn AS "playlist_urn!"
