WITH usable AS MATERIALIZED (
    SELECT DISTINCT ON (connection.soundcloud_user_id)
           connection.soundcloud_user_id AS user_id,
           connection.oauth_app_id::text AS app,
           EXISTS (
               SELECT 1
               FROM subscriptions AS subscription
               WHERE subscription.user_urn = connection.soundcloud_user_id
                 AND subscription.exp_date > extract(epoch FROM now())::bigint
           ) AS star
    FROM soundcloud_connections AS connection
    JOIN oauth_apps AS app ON app.id = connection.oauth_app_id
    WHERE app.active
      AND connection.last_refresh_error_kind IS DISTINCT FROM 'reauthorization_required'
      AND (connection.retry_at IS NULL OR connection.retry_at <= now())
    ORDER BY connection.soundcloud_user_id,
             EXISTS (
                 SELECT 1
                 FROM sync_queue_action_pauses AS pause
                 WHERE pause.oauth_app_id = connection.oauth_app_id
                   AND pause.reason = 'subscription_required'
             ),
             (connection.expires_at > now()) DESC,
             connection.updated_at DESC
),
waiting AS MATERIALIZED (
    SELECT head.id, head.next_run_at, head.created_at, head.paced, usable.star
    FROM usable
    CROSS JOIN LATERAL (
        SELECT ordered.id, ordered.next_run_at, ordered.created_at, ordered.paced
        FROM (
            SELECT queued.id,
                   queued.next_run_at,
                   queued.created_at,
                   CASE WHEN queued.action_type = ANY ($4::text[]) THEN queued.action_type END AS paced,
                   row_number() OVER (
                       PARTITION BY CASE
                           WHEN queued.action_type = ANY ($4::text[]) THEN queued.action_type
                       END
                       ORDER BY queued.next_run_at, queued.created_at, queued.id
                   ) AS turn
            FROM sync_queue AS queued
            WHERE queued.user_id = usable.user_id
              AND queued.dead = false
              AND queued.next_run_at <= now()
              AND (
                  queued.lease_id IS NULL
                  OR queued.locked_at < now() - $1::bigint * interval '1 millisecond'
              )
              AND NOT ((usable.app || ':' || queued.action_type) = ANY ($3::text[]))
              AND NOT (
                  queued.cancelled
                  AND queued.lease_id IS NULL
                  AND queued.remote_attempted_generation IS NULL
                  AND queued.remote_completed_generation IS NULL
              )
              AND NOT (
                  queued.exec_generation IS DISTINCT FROM queued.generation
                  AND (queued.exec_generation IS NOT NULL OR queued.intent_v > 0)
              )
              AND NOT EXISTS (
                  SELECT 1
                  FROM sync_queue AS earlier
                  WHERE earlier.dead = false
                    AND earlier.user_id = queued.user_id
                    AND earlier.target_urn = queued.target_urn
                    AND (earlier.created_at, earlier.id) < (queued.created_at, queued.id)
                    AND (
                        earlier.action_type <> ALL ($5::text[])
                        OR queued.action_type = ANY ($5::text[])
                    )
              )
        ) AS ordered
        WHERE ordered.paced IS NULL OR ordered.turn = 1
        ORDER BY ordered.next_run_at, ordered.created_at, ordered.id
        LIMIT $2
    ) AS head
),
due AS MATERIALIZED (
    SELECT ranked.id, ranked.star
    FROM (
        SELECT waiting.id,
               waiting.next_run_at,
               waiting.created_at,
               waiting.paced,
               waiting.star,
               row_number() OVER (
                   PARTITION BY waiting.paced
                   ORDER BY waiting.star = $6 DESC,
                            waiting.created_at > now() - interval '7 days' DESC,
                            waiting.next_run_at,
                            waiting.created_at,
                            waiting.id
               ) AS turn
        FROM waiting
    ) AS ranked
    WHERE ranked.paced IS NULL OR ranked.turn = 1
    ORDER BY ranked.star = $6 DESC,
             ranked.created_at > now() - interval '7 days' DESC,
             ranked.next_run_at,
             ranked.created_at,
             ranked.id
    LIMIT $2
),
candidates AS (
    SELECT queued.id
    FROM sync_queue AS queued
    WHERE queued.id = ANY (ARRAY(SELECT id FROM due))
      AND queued.dead = false
      AND queued.next_run_at <= now()
      AND (
          queued.lease_id IS NULL
          OR queued.locked_at < now() - $1::bigint * interval '1 millisecond'
      )
      AND NOT (
          queued.cancelled
          AND queued.lease_id IS NULL
          AND queued.remote_attempted_generation IS NULL
          AND queued.remote_completed_generation IS NULL
      )
      AND NOT (
          queued.exec_generation IS DISTINCT FROM queued.generation
          AND (queued.exec_generation IS NOT NULL OR queued.intent_v > 0)
      )
    FOR UPDATE OF queued SKIP LOCKED
)
UPDATE sync_queue AS queued
SET lease_id = gen_random_uuid(),
    lease_generation = queued.generation,
    exec_generation = queued.generation,
    locked_at = now()
FROM candidates
WHERE queued.id = candidates.id
RETURNING queued.id,
          queued.user_id,
          queued.action_type,
          queued.target_urn,
          queued.payload,
          queued.retry_count,
          queued.generation,
          queued.lease_id AS "lease_id!",
          queued.lease_generation AS "lease_generation!",
          queued.remote_attempted_generation,
          queued.remote_completed_generation,
          queued.remote_result,
          queued.created_at,
          (SELECT due.star FROM due WHERE due.id = queued.id) AS "star!",
          CASE queued.action_type
              WHEN 'track_update' THEN (
                  SELECT track.sc_desired FROM tracks AS track
                  WHERE track.urn = queued.target_urn AND track.deleted_at IS NULL
              )
              WHEN 'playlist_update' THEN (
                  SELECT playlist.sc_desired FROM playlists AS playlist
                  WHERE playlist.urn = queued.target_urn AND playlist.deleted_at IS NULL
              )
          END AS desired
