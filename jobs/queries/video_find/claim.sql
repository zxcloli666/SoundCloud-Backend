WITH due AS (
    SELECT track.sc_track_id
    FROM star_track_tiers AS tier
    JOIN tracks AS track ON track.sc_track_id = tier.sc_track_id
    LEFT JOIN track_video_dispatch AS dispatch ON dispatch.sc_track_id = track.sc_track_id
    WHERE tier.tier = 1
      AND track.sharing = 'public'
      AND track.storage_state = 'ok'
      AND track.superseded_by IS NULL
      AND track.deleted_at IS NULL
      AND NOT EXISTS (
          SELECT 1
          FROM track_videos AS video
          WHERE video.sc_track_id = track.sc_track_id
            AND video.kind IN ('clip', 'loop')
      )
      AND (
          dispatch.sc_track_id IS NULL
          OR (dispatch.stage NOT IN ('dispatched', 'pending_vlm', 'found')
              AND dispatch.next_at <= now())
      )
    ORDER BY COALESCE(track.play_count_sc, 0) + 10 * COALESCE(track.likes_count_sc, 0) DESC,
             track.sc_track_id
    LIMIT $1
), held AS (
    INSERT INTO track_video_dispatch AS dispatch (sc_track_id, stage, attempts, next_at)
    SELECT due.sc_track_id, 'dispatched', 1, now()
    FROM due
    ON CONFLICT (sc_track_id) DO UPDATE
    SET stage = 'dispatched',
        attempts = dispatch.attempts + 1,
        payload = NULL,
        updated_at = now()
    RETURNING dispatch.sc_track_id, dispatch.attempts
)
SELECT track.sc_track_id AS "sc_track_id!",
       held.attempts AS "attempt!",
       track.title AS "title!",
       COALESCE(artist.name, track.metadata_artist, track.uploader_username, '') AS "artist!",
       track.isrc,
       track.duration_ms
FROM held
JOIN tracks AS track ON track.sc_track_id = held.sc_track_id
LEFT JOIN artists AS artist ON artist.id = track.primary_artist_id
