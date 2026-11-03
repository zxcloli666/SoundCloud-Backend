SELECT count(*) AS "revivable!"
FROM sync_queue
WHERE user_id = ANY ($1)
  AND dead
  AND failed_at < $4::timestamptz - interval '2 minutes'
  AND failed_at > $4::timestamptz - interval '30 days'
  AND ($2::text IS NULL OR target_urn = $2)
  AND (
      action_type NOT IN (
          'comment', 'like_track', 'unlike_track', 'like_playlist', 'unlike_playlist',
          'follow_user', 'unfollow_user', 'playlist_membership'
      )
      OR (action_type = 'playlist_membership' AND target_urn = ANY ($3))
  )
