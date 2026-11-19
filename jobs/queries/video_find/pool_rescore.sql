WITH oldest AS (
    SELECT sc_track_id
    FROM track_video_pool
    ORDER BY scored_at, sc_track_id
    LIMIT $1
), fresh AS (
    SELECT oldest.sc_track_id,
           COALESCE(track.play_count_sc, 0) + 10 * COALESCE(track.likes_count_sc, 0) AS score
    FROM oldest
    JOIN star_track_tiers AS tier ON tier.sc_track_id = oldest.sc_track_id AND tier.tier = 1
    JOIN tracks AS track ON track.sc_track_id = oldest.sc_track_id
    WHERE track.sharing = 'public'
      AND track.storage_state = 'ok'
      AND track.superseded_by IS NULL
      AND track.deleted_at IS NULL
      AND NOT EXISTS (
          SELECT 1 FROM track_video_dispatch AS dispatch WHERE dispatch.sc_track_id = oldest.sc_track_id
      )
      AND NOT EXISTS (
          SELECT 1
          FROM track_videos AS video
          WHERE video.sc_track_id = oldest.sc_track_id
            AND video.kind IN ('clip', 'loop')
      )
), gone AS (
    DELETE FROM track_video_pool AS pool
    USING oldest
    WHERE pool.sc_track_id = oldest.sc_track_id
      AND NOT EXISTS (SELECT 1 FROM fresh WHERE fresh.sc_track_id = oldest.sc_track_id)
)
UPDATE track_video_pool AS pool
SET score = fresh.score,
    scored_at = now()
FROM fresh
WHERE pool.sc_track_id = fresh.sc_track_id
