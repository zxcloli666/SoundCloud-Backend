WITH star AS MATERIALIZED (
    SELECT regexp_replace(subscription.user_urn, '^soundcloud:users:', '') AS id
    FROM subscriptions AS subscription
    WHERE subscription.exp_date > extract(epoch FROM now())::bigint
    UNION
    SELECT flag.sc_user_id
    FROM video_flags AS flag
    WHERE flag.kind = 'account'
), wanted AS MATERIALIZED (
    SELECT superlike.sc_track_id, 0 AS rank
    FROM star
    JOIN user_superlikes AS superlike
      ON superlike.user_id IN (star.id, 'soundcloud:users:' || star.id)
    UNION ALL
    SELECT track.sc_track_id, 1
    FROM star
    JOIN tracks AS track ON track.uploader_sc_user_id = star.id
    UNION ALL
    SELECT track.sc_track_id, 2
    FROM video_flags AS flag
    JOIN tracks AS track ON track.uploader_sc_user_id = flag.sc_user_id
    WHERE flag.kind = 'artist'
), picked AS (
    SELECT wanted.sc_track_id, min(wanted.rank) AS rank
    FROM wanted
    JOIN tracks AS track ON track.sc_track_id = wanted.sc_track_id
    WHERE track.sharing = 'public'
      AND track.storage_state = 'ok'
      AND track.superseded_by IS NULL
      AND track.deleted_at IS NULL
      AND track.artwork_url IS NOT NULL
      AND track.duration_ms > 0
      AND NOT EXISTS (
          SELECT 1 FROM track_videos AS video WHERE video.sc_track_id = track.sc_track_id
      )
      AND NOT EXISTS (
          SELECT 1 FROM track_video_gen AS gen WHERE gen.sc_track_id = track.sc_track_id
      )
      AND NOT EXISTS (
          SELECT 1
          FROM track_video_dispatch AS dispatch
          WHERE dispatch.sc_track_id = track.sc_track_id
            AND dispatch.stage IN ('dispatched', 'pending_vlm')
      )
    GROUP BY wanted.sc_track_id
    ORDER BY rank, wanted.sc_track_id
    LIMIT $1
)
INSERT INTO track_video_gen (sc_track_id, stage)
SELECT picked.sc_track_id, 'plan'
FROM picked
ON CONFLICT (sc_track_id) DO NOTHING
