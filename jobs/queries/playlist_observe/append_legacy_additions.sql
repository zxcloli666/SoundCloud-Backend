WITH edit AS (
    SELECT max(intent.queue_created_at) AS edited_at
    FROM playlist_legacy_membership_intents AS intent
    WHERE intent.playlist_urn = $1
      AND intent.classification NOT IN ('resolved', 'abandoned')
), first_read AS (
    SELECT observation.sc_last_modified
    FROM playlist_remote_observations AS observation
    WHERE observation.playlist_urn = $1
      AND observation.snapshot_id IS NOT NULL
      AND observation.sc_last_modified IS NOT NULL
    ORDER BY observation.observed_at, observation.id
    LIMIT 1
), listed AS (
    SELECT entry.sc_track_id,
           entry.position,
           lag(entry.sc_track_id) OVER (ORDER BY entry.position) AS left_anchor,
           lead(entry.sc_track_id) OVER (ORDER BY entry.position) AS right_anchor
    FROM unnest($3::text[]) WITH ORDINALITY AS entry (sc_track_id, position)
), addition AS (
    SELECT listed.sc_track_id,
           listed.left_anchor,
           listed.right_anchor,
           row_number() OVER (ORDER BY listed.position) AS ordinal
    FROM listed
    JOIN tracks AS track
      ON track.sc_track_id = listed.sc_track_id
     AND track.deleted_at IS NULL
    WHERE EXISTS (
            SELECT 1
            FROM edit, first_read
            WHERE first_read.sc_last_modified <= edit.edited_at
        )
      AND NOT EXISTS (
            SELECT 1
            FROM playlist_remote_observations AS observation
            JOIN playlist_remote_snapshot_tracks AS seen
              ON seen.snapshot_id = observation.snapshot_id
            WHERE observation.playlist_urn = $1
              AND seen.sc_track_id = listed.sc_track_id
        )
), base AS (
    SELECT GREATEST(state.last_operation_sequence, coalesce(max(operation.sequence), 0)) AS sequence,
           state.projection_revision,
           state.baseline_generation,
           state.baseline_observation_id
    FROM playlist_membership_state AS state
    LEFT JOIN playlist_membership_operations AS operation
      ON operation.playlist_urn = state.playlist_urn
    WHERE state.playlist_urn = $1
    GROUP BY state.playlist_urn
), bumped AS (
    UPDATE playlist_membership_state AS state
    SET last_operation_sequence = base.sequence + added.total,
        projection_revision = base.projection_revision + added.total,
        updated_at = clock_timestamp()
    FROM base, (SELECT count(*) AS total FROM addition) AS added
    WHERE state.playlist_urn = $1
      AND added.total > 0
)
INSERT INTO playlist_membership_operations (
    operation_id, playlist_urn, sequence, actor_sc_user_id, idempotency_key, request_fingerprint,
    base_baseline_generation, base_observation_id, expected_projection_revision,
    accepted_projection_revision, kind, track_id, left_anchor_track_id, right_anchor_track_id, boundary
)
SELECT gen_random_uuid(),
       $1,
       base.sequence + addition.ordinal,
       $2,
       md5('playlist-legacy-addition:' || $1 || ':' || addition.sc_track_id)::uuid,
       sha256(convert_to($1 || ':' || addition.sc_track_id, 'UTF8')),
       base.baseline_generation,
       base.baseline_observation_id,
       base.projection_revision + addition.ordinal - 1,
       base.projection_revision + addition.ordinal,
       'add',
       addition.sc_track_id,
       addition.left_anchor,
       addition.right_anchor,
       CASE WHEN addition.left_anchor IS NULL AND addition.right_anchor IS NULL THEN 'back' END
FROM base, addition
RETURNING sequence
