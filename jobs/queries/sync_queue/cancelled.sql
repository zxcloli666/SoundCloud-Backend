SELECT queued.id,
       opposite.id AS "opposite_id?"
FROM sync_queue AS queued
LEFT JOIN sync_queue AS opposite
       ON opposite.user_id = queued.user_id
      AND opposite.target_urn = queued.target_urn
      AND opposite.action_type = CASE queued.action_type
          WHEN 'like_track' THEN 'unlike_track'
          WHEN 'unlike_track' THEN 'like_track'
          WHEN 'like_playlist' THEN 'unlike_playlist'
          WHEN 'unlike_playlist' THEN 'like_playlist'
          WHEN 'follow_user' THEN 'unfollow_user'
          WHEN 'unfollow_user' THEN 'follow_user'
      END
WHERE queued.cancelled
  AND queued.lease_id IS NULL
  AND queued.remote_attempted_generation IS NULL
  AND queued.remote_completed_generation IS NULL
ORDER BY queued.id
LIMIT $1
