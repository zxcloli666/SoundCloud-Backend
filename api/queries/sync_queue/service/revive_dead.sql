WITH picked AS (
    SELECT id, action_type, target_urn
    FROM sync_queue
    WHERE user_id = ANY ($1)
      AND dead
      AND failed_at < now() - interval '2 minutes'
      AND failed_at > now() - interval '30 days'
      AND action_type NOT IN (
          'comment', 'like_track', 'unlike_track', 'like_playlist', 'unlike_playlist',
          'follow_user', 'unfollow_user'
      )
      AND ($2::text IS NULL OR target_urn = $2)
    FOR UPDATE SKIP LOCKED
),
revived AS (
    UPDATE sync_queue AS queued
    SET dead = false,
        failed_at = NULL,
        retry_count = 0,
        last_error = NULL,
        next_run_at = now()
    FROM picked
    WHERE queued.id = picked.id
      AND picked.action_type <> 'playlist_membership'
    RETURNING queued.id
),
dropped AS (
    DELETE FROM sync_queue AS queued
    USING picked
    WHERE queued.id = picked.id
      AND picked.action_type = 'playlist_membership'
    RETURNING queued.target_urn
)
SELECT (SELECT count(*) FROM revived) + (SELECT count(*) FROM dropped) AS "revived!",
       ARRAY(SELECT target_urn FROM dropped) AS "reobserve!"
