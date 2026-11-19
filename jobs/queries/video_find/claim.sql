WITH ranked AS (
    (
        SELECT pool.sc_track_id, pool.score
        FROM track_video_pool AS pool
        JOIN star_track_tiers AS tier ON tier.sc_track_id = pool.sc_track_id AND tier.tier = 1
        JOIN tracks AS track ON track.sc_track_id = pool.sc_track_id
        WHERE track.sharing = 'public'
          AND track.storage_state = 'ok'
          AND track.superseded_by IS NULL
          AND track.deleted_at IS NULL
          AND NOT EXISTS (
              SELECT 1 FROM track_video_dispatch AS dispatch WHERE dispatch.sc_track_id = pool.sc_track_id
          )
          AND NOT EXISTS (
              SELECT 1
              FROM track_videos AS video
              WHERE video.sc_track_id = pool.sc_track_id
                AND video.kind IN ('clip', 'loop')
          )
        ORDER BY pool.score DESC, pool.sc_track_id
        LIMIT $1
    )
    UNION ALL
    (
        SELECT track.sc_track_id,
               COALESCE(track.play_count_sc, 0) + 10 * COALESCE(track.likes_count_sc, 0)
        FROM track_video_dispatch AS dispatch
        JOIN star_track_tiers AS tier ON tier.sc_track_id = dispatch.sc_track_id AND tier.tier = 1
        JOIN tracks AS track ON track.sc_track_id = dispatch.sc_track_id
        WHERE dispatch.stage NOT IN ('dispatched', 'pending_vlm', 'vlm_dispatched', 'found')
          AND dispatch.next_at <= now()
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
        ORDER BY 2 DESC, track.sc_track_id
        LIMIT $1
    )
), due AS (
    SELECT sc_track_id
    FROM ranked
    ORDER BY score DESC, sc_track_id
    LIMIT $1
), taken AS (
    DELETE FROM track_video_pool
    WHERE sc_track_id IN (SELECT sc_track_id FROM due)
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
