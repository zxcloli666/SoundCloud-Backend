INSERT INTO track_video_pool (sc_track_id, score)
SELECT tier.sc_track_id,
       COALESCE(track.play_count_sc, 0) + 10 * COALESCE(track.likes_count_sc, 0)
FROM star_track_tiers AS tier
JOIN tracks AS track ON track.sc_track_id = tier.sc_track_id
WHERE tier.tier = 1
  AND track.sharing = 'public'
  AND track.storage_state = 'ok'
  AND track.superseded_by IS NULL
  AND track.deleted_at IS NULL
  AND NOT EXISTS (SELECT 1 FROM track_video_pool AS pool WHERE pool.sc_track_id = tier.sc_track_id)
  AND NOT EXISTS (SELECT 1 FROM track_video_dispatch AS dispatch WHERE dispatch.sc_track_id = tier.sc_track_id)
  AND NOT EXISTS (
      SELECT 1
      FROM track_videos AS video
      WHERE video.sc_track_id = tier.sc_track_id
        AND video.kind IN ('clip', 'loop')
  )
LIMIT $1
ON CONFLICT (sc_track_id) DO NOTHING
