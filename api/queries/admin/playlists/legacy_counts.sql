SELECT classification AS "classification!",
       count(*) AS "count!"
FROM playlist_legacy_membership_intents
WHERE resolved_at IS NULL
GROUP BY classification
ORDER BY classification
