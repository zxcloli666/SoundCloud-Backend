DELETE FROM track_comments c
WHERE c.sc_track_id = $1
  AND c.created_at <= $2 AND (c.synced_at IS NULL OR c.synced_at <= $2)
  AND (
      c.sc_comment_id IS NULL
      OR NOT EXISTS (
          SELECT 1 FROM catalog_collection_seen s
          WHERE s.subject_id = $1 AND s.collection = 'track-comments' AND s.scope = 'public'
            AND s.snapshot_id = $3 AND s.entity_key = c.sc_comment_id
      )
  )
  AND NOT EXISTS (
      SELECT 1 FROM sync_queue q
      WHERE q.action_type = 'comment'
        AND q.target_urn = 'soundcloud:tracks:' || c.sc_track_id
        AND (q.user_id = c.user_urn OR 'soundcloud:users:' || q.user_id = c.user_urn)
  )
