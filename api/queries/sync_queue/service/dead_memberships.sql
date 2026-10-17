SELECT target_urn
FROM sync_queue
WHERE user_id = ANY ($1)
  AND action_type = 'playlist_membership'
  AND dead
  AND failed_at < now() - interval '2 minutes'
  AND failed_at > now() - interval '30 days'
  AND ($2::text IS NULL OR target_urn = $2)
ORDER BY target_urn
