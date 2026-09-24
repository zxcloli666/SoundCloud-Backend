DO $$
DECLARE
    membership_table regclass;
    duplicate_membership text;
    has_negative_position boolean;
BEGIN
    IF to_regclass('playlist_track_projection') IS NOT NULL THEN
        IF to_regclass('playlist_tracks') IS NOT NULL THEN
            RAISE EXCEPTION 'both playlist_tracks and playlist_track_projection exist';
        END IF;
        membership_table := to_regclass('playlist_track_projection');
    ELSE
        membership_table := to_regclass('playlist_tracks');
    END IF;

    IF membership_table IS NULL THEN
        RAISE EXCEPTION 'playlist membership table is missing';
    END IF;

    EXECUTE format(
        'SELECT playlist_urn || '':'' || sc_track_id
           FROM %s
          GROUP BY playlist_urn, sc_track_id
         HAVING count(*) > 1
          ORDER BY playlist_urn, sc_track_id
          LIMIT 1',
        membership_table
    ) INTO duplicate_membership;

    IF duplicate_membership IS NOT NULL THEN
        RAISE EXCEPTION USING
            ERRCODE = 'unique_violation',
            MESSAGE = format(
                'duplicate playlist membership must be repaired before migration: %s',
                duplicate_membership
            );
    END IF;

    EXECUTE format(
        'SELECT EXISTS (SELECT 1 FROM %s WHERE position < 0)',
        membership_table
    ) INTO STRICT has_negative_position;

    IF has_negative_position THEN
        RAISE EXCEPTION 'negative playlist position must be repaired before migration';
    END IF;

    IF membership_table = to_regclass('playlist_tracks') THEN
        ALTER TABLE playlist_tracks RENAME TO playlist_track_projection;
    END IF;
END $$;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conrelid = 'playlist_track_projection'::regclass
          AND conname = 'playlist_tracks_pkey'
    ) THEN
        ALTER TABLE playlist_track_projection
            RENAME CONSTRAINT playlist_tracks_pkey TO playlist_track_projection_pkey;
    END IF;
END $$;

ALTER INDEX IF EXISTS playlist_tracks_track_idx
    RENAME TO playlist_track_projection_track_legacy_idx;

DROP INDEX IF EXISTS playlist_tracks_playlist_idx;

CREATE UNIQUE INDEX IF NOT EXISTS playlist_track_projection_member_uq
    ON playlist_track_projection (playlist_urn, sc_track_id);

CREATE INDEX IF NOT EXISTS playlist_track_projection_track_idx
    ON playlist_track_projection (sc_track_id, playlist_urn);

DROP INDEX IF EXISTS playlist_track_projection_track_legacy_idx;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conrelid = 'playlist_track_projection'::regclass
          AND conname = 'playlist_track_projection_position_nonnegative'
    ) THEN
        ALTER TABLE playlist_track_projection
            ADD CONSTRAINT playlist_track_projection_position_nonnegative
            CHECK (position >= 0) NOT VALID;
    END IF;

    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conrelid = 'playlist_track_projection'::regclass
          AND conname = 'playlist_track_projection_playlist_fkey'
    ) THEN
        ALTER TABLE playlist_track_projection
            ADD CONSTRAINT playlist_track_projection_playlist_fkey
            FOREIGN KEY (playlist_urn) REFERENCES playlists (urn) ON DELETE CASCADE
            NOT VALID;
    END IF;
END $$;

ALTER TABLE playlist_track_projection
    VALIDATE CONSTRAINT playlist_track_projection_position_nonnegative;

ALTER TABLE playlist_track_projection
    VALIDATE CONSTRAINT playlist_track_projection_playlist_fkey;

CREATE TABLE IF NOT EXISTS playlist_remote_snapshots (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    playlist_urn text NOT NULL REFERENCES playlists (urn) ON DELETE CASCADE,
    content_fingerprint bytea NOT NULL,
    track_count integer NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT playlist_remote_snapshots_fingerprint_size
        CHECK (octet_length(content_fingerprint) = 32),
    CONSTRAINT playlist_remote_snapshots_track_count_nonnegative
        CHECK (track_count >= 0),
    CONSTRAINT playlist_remote_snapshots_playlist_identity_uq
        UNIQUE (playlist_urn, id),
    CONSTRAINT playlist_remote_snapshots_content_uq
        UNIQUE (playlist_urn, content_fingerprint)
);

CREATE TABLE IF NOT EXISTS playlist_remote_snapshot_tracks (
    snapshot_id uuid NOT NULL REFERENCES playlist_remote_snapshots (id) ON DELETE CASCADE,
    position integer NOT NULL,
    sc_track_id text NOT NULL,
    PRIMARY KEY (snapshot_id, position),
    CONSTRAINT playlist_remote_snapshot_tracks_position_nonnegative
        CHECK (position >= 0),
    CONSTRAINT playlist_remote_snapshot_tracks_member_uq
        UNIQUE (snapshot_id, sc_track_id)
);

CREATE TABLE IF NOT EXISTS playlist_remote_observations (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    playlist_urn text NOT NULL REFERENCES playlists (urn) ON DELETE CASCADE,
    snapshot_id uuid,
    authority text NOT NULL,
    outcome text NOT NULL,
    pagination_complete boolean NOT NULL DEFAULT false,
    all_items_identified boolean NOT NULL DEFAULT false,
    declared_track_count integer,
    observed_track_count integer NOT NULL DEFAULT 0,
    sc_last_modified timestamptz,
    observed_at timestamptz NOT NULL DEFAULT now(),
    retry_at timestamptz,
    error_kind text,
    write_eligible boolean GENERATED ALWAYS AS (
        authority = 'owner'
        AND outcome = 'complete'
        AND snapshot_id IS NOT NULL
        AND pagination_complete
        AND all_items_identified
        AND coalesce(declared_track_count = observed_track_count, false)
    ) STORED,
    CONSTRAINT playlist_remote_observations_authority_valid
        CHECK (authority IN ('owner', 'public')),
    CONSTRAINT playlist_remote_observations_outcome_valid
        CHECK (outcome IN (
            'complete',
            'incomplete',
            'malformed',
            'not_found',
            'unauthorized',
            'rate_limited',
            'transport'
        )),
    CONSTRAINT playlist_remote_observations_declared_count_nonnegative
        CHECK (declared_track_count IS NULL OR declared_track_count >= 0),
    CONSTRAINT playlist_remote_observations_observed_count_nonnegative
        CHECK (observed_track_count >= 0),
    CONSTRAINT playlist_remote_observations_complete_evidence
        CHECK (
            outcome <> 'complete'
            OR (
                snapshot_id IS NOT NULL
                AND pagination_complete
                AND all_items_identified
                AND declared_track_count = observed_track_count
                AND error_kind IS NULL
            )
        ),
    CONSTRAINT playlist_remote_observations_failure_evidence
        CHECK (outcome = 'complete' OR error_kind IS NOT NULL),
    CONSTRAINT playlist_remote_observations_playlist_identity_uq
        UNIQUE (playlist_urn, id),
    CONSTRAINT playlist_remote_observations_snapshot_fkey
        FOREIGN KEY (playlist_urn, snapshot_id)
        REFERENCES playlist_remote_snapshots (playlist_urn, id)
);

CREATE INDEX IF NOT EXISTS playlist_remote_observations_history_idx
    ON playlist_remote_observations (playlist_urn, observed_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS playlist_remote_observations_write_eligible_idx
    ON playlist_remote_observations (playlist_urn, observed_at DESC, id DESC)
    WHERE write_eligible;

CREATE TABLE IF NOT EXISTS playlist_membership_state (
    playlist_urn text PRIMARY KEY REFERENCES playlists (urn) ON DELETE CASCADE,
    baseline_generation bigint NOT NULL DEFAULT 0,
    baseline_observation_id uuid,
    latest_observation_id uuid,
    projection_revision bigint NOT NULL DEFAULT 0,
    projection_track_count integer NOT NULL DEFAULT 0,
    last_operation_sequence bigint NOT NULL DEFAULT 0,
    committed_operation_sequence bigint NOT NULL DEFAULT 0,
    reconcile_generation bigint NOT NULL DEFAULT 0,
    sync_status text NOT NULL DEFAULT 'unhydrated',
    conflict_code text,
    candidate_fingerprint bytea,
    next_reconcile_at timestamptz,
    last_error text,
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT playlist_membership_state_baseline_generation_nonnegative
        CHECK (baseline_generation >= 0),
    CONSTRAINT playlist_membership_state_projection_revision_nonnegative
        CHECK (projection_revision >= 0),
    CONSTRAINT playlist_membership_state_projection_count_nonnegative
        CHECK (projection_track_count >= 0),
    CONSTRAINT playlist_membership_state_operation_sequence_valid
        CHECK (
            last_operation_sequence >= 0
            AND committed_operation_sequence >= 0
            AND committed_operation_sequence <= last_operation_sequence
        ),
    CONSTRAINT playlist_membership_state_reconcile_generation_nonnegative
        CHECK (reconcile_generation >= 0),
    CONSTRAINT playlist_membership_state_status_valid
        CHECK (sync_status IN (
            'unhydrated',
            'legacy_review',
            'clean',
            'pending',
            'shadow_ready',
            'conflict',
            'auth_required',
            'retry_wait'
        )),
    CONSTRAINT playlist_membership_state_baseline_complete
        CHECK (
            (baseline_generation = 0 AND baseline_observation_id IS NULL)
            OR
            (baseline_generation > 0 AND baseline_observation_id IS NOT NULL)
        ),
    CONSTRAINT playlist_membership_state_conflict_complete
        CHECK ((sync_status = 'conflict') = (conflict_code IS NOT NULL)),
    CONSTRAINT playlist_membership_state_candidate_fingerprint_size
        CHECK (
            candidate_fingerprint IS NULL
            OR octet_length(candidate_fingerprint) = 32
        ),
    CONSTRAINT playlist_membership_state_baseline_observation_fkey
        FOREIGN KEY (playlist_urn, baseline_observation_id)
        REFERENCES playlist_remote_observations (playlist_urn, id),
    CONSTRAINT playlist_membership_state_latest_observation_fkey
        FOREIGN KEY (playlist_urn, latest_observation_id)
        REFERENCES playlist_remote_observations (playlist_urn, id)
);

CREATE INDEX IF NOT EXISTS playlist_membership_state_due_idx
    ON playlist_membership_state (next_reconcile_at, playlist_urn)
    WHERE sync_status IN (
        'unhydrated',
        'legacy_review',
        'pending',
        'retry_wait',
        'auth_required'
    );

CREATE INDEX IF NOT EXISTS playlist_membership_state_conflict_idx
    ON playlist_membership_state (updated_at DESC, playlist_urn)
    WHERE sync_status = 'conflict';

CREATE TABLE IF NOT EXISTS playlist_membership_operations (
    operation_id uuid PRIMARY KEY,
    playlist_urn text NOT NULL REFERENCES playlist_membership_state (playlist_urn) ON DELETE CASCADE,
    sequence bigint NOT NULL,
    actor_sc_user_id text NOT NULL,
    idempotency_key uuid NOT NULL,
    request_fingerprint bytea NOT NULL,
    base_baseline_generation bigint NOT NULL,
    base_observation_id uuid NOT NULL,
    expected_projection_revision bigint NOT NULL,
    accepted_projection_revision bigint NOT NULL,
    kind text NOT NULL,
    track_id text,
    left_anchor_track_id text,
    right_anchor_track_id text,
    boundary text,
    ordered_track_ids text[],
    outcome text NOT NULL DEFAULT 'pending',
    conflict_code text,
    created_at timestamptz NOT NULL DEFAULT now(),
    resolved_at timestamptz,
    CONSTRAINT playlist_membership_operations_sequence_positive
        CHECK (sequence > 0),
    CONSTRAINT playlist_membership_operations_fingerprint_size
        CHECK (octet_length(request_fingerprint) = 32),
    CONSTRAINT playlist_membership_operations_base_generation_positive
        CHECK (base_baseline_generation > 0),
    CONSTRAINT playlist_membership_operations_projection_revision_valid
        CHECK (
            expected_projection_revision >= 0
            AND accepted_projection_revision = expected_projection_revision + 1
        ),
    CONSTRAINT playlist_membership_operations_kind_valid
        CHECK (kind IN ('add', 'remove', 'move', 'reorder')),
    CONSTRAINT playlist_membership_operations_boundary_valid
        CHECK (boundary IS NULL OR boundary IN ('front', 'back')),
    CONSTRAINT playlist_membership_operations_shape_valid
        CHECK (
            (
                kind IN ('add', 'move')
                AND track_id IS NOT NULL
                AND ordered_track_ids IS NULL
                AND (
                    (
                        boundary IS NOT NULL
                        AND left_anchor_track_id IS NULL
                        AND right_anchor_track_id IS NULL
                    )
                    OR
                    (
                        boundary IS NULL
                        AND (
                            left_anchor_track_id IS NOT NULL
                            OR right_anchor_track_id IS NOT NULL
                        )
                    )
                )
            )
            OR
            (
                kind = 'remove'
                AND track_id IS NOT NULL
                AND left_anchor_track_id IS NULL
                AND right_anchor_track_id IS NULL
                AND boundary IS NULL
                AND ordered_track_ids IS NULL
            )
            OR
            (
                kind = 'reorder'
                AND track_id IS NULL
                AND left_anchor_track_id IS NULL
                AND right_anchor_track_id IS NULL
                AND boundary IS NULL
                AND cardinality(ordered_track_ids) > 0
            )
        ),
    CONSTRAINT playlist_membership_operations_anchor_target_distinct
        CHECK (
            track_id IS NULL
            OR (
                track_id IS DISTINCT FROM left_anchor_track_id
                AND track_id IS DISTINCT FROM right_anchor_track_id
            )
        ),
    CONSTRAINT playlist_membership_operations_outcome_valid
        CHECK (outcome IN ('pending', 'committed', 'conflict', 'superseded')),
    CONSTRAINT playlist_membership_operations_resolution_complete
        CHECK ((outcome = 'pending') = (resolved_at IS NULL)),
    CONSTRAINT playlist_membership_operations_conflict_complete
        CHECK ((outcome = 'conflict') = (conflict_code IS NOT NULL)),
    CONSTRAINT playlist_membership_operations_sequence_uq
        UNIQUE (playlist_urn, sequence),
    CONSTRAINT playlist_membership_operations_idempotency_uq
        UNIQUE (playlist_urn, idempotency_key),
    CONSTRAINT playlist_membership_operations_base_observation_fkey
        FOREIGN KEY (playlist_urn, base_observation_id)
        REFERENCES playlist_remote_observations (playlist_urn, id)
);

CREATE INDEX IF NOT EXISTS playlist_membership_operations_pending_idx
    ON playlist_membership_operations (playlist_urn, sequence)
    WHERE resolved_at IS NULL;

CREATE INDEX IF NOT EXISTS playlist_membership_operations_outcome_idx
    ON playlist_membership_operations (outcome, created_at, playlist_urn, sequence)
    WHERE outcome IN ('pending', 'conflict');

CREATE TABLE IF NOT EXISTS playlist_reconcile_runs (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    playlist_urn text NOT NULL REFERENCES playlist_membership_state (playlist_urn) ON DELETE CASCADE,
    reconcile_generation bigint NOT NULL,
    job_id uuid NOT NULL,
    job_generation bigint NOT NULL,
    mode text NOT NULL DEFAULT 'shadow',
    captured_baseline_generation bigint NOT NULL,
    captured_through_operation_sequence bigint NOT NULL,
    observation_id uuid,
    candidate_fingerprint bytea,
    decision text NOT NULL DEFAULT 'started',
    reason text,
    started_at timestamptz NOT NULL DEFAULT now(),
    completed_at timestamptz,
    CONSTRAINT playlist_reconcile_runs_reconcile_generation_positive
        CHECK (reconcile_generation > 0),
    CONSTRAINT playlist_reconcile_runs_job_generation_positive
        CHECK (job_generation > 0),
    CONSTRAINT playlist_reconcile_runs_baseline_generation_nonnegative
        CHECK (captured_baseline_generation >= 0),
    CONSTRAINT playlist_reconcile_runs_operation_sequence_nonnegative
        CHECK (captured_through_operation_sequence >= 0),
    CONSTRAINT playlist_reconcile_runs_mode_shadow
        CHECK (mode = 'shadow'),
    CONSTRAINT playlist_reconcile_runs_candidate_fingerprint_size
        CHECK (
            candidate_fingerprint IS NULL
            OR octet_length(candidate_fingerprint) = 32
        ),
    CONSTRAINT playlist_reconcile_runs_decision_valid
        CHECK (decision IN (
            'started',
            'clean',
            'legacy_equal',
            'remote_superset',
            'local_superset',
            'order_only',
            'membership_diverged',
            'shadow_ready',
            'conflict',
            'auth_required',
            'retry_wait',
            'incomplete',
            'superseded'
        )),
    CONSTRAINT playlist_reconcile_runs_completion_valid
        CHECK ((decision = 'started') = (completed_at IS NULL)),
    CONSTRAINT playlist_reconcile_runs_generation_uq
        UNIQUE (playlist_urn, reconcile_generation),
    CONSTRAINT playlist_reconcile_runs_job_uq
        UNIQUE (job_id, job_generation),
    CONSTRAINT playlist_reconcile_runs_observation_fkey
        FOREIGN KEY (playlist_urn, observation_id)
        REFERENCES playlist_remote_observations (playlist_urn, id)
);

CREATE INDEX IF NOT EXISTS playlist_reconcile_runs_history_idx
    ON playlist_reconcile_runs (playlist_urn, started_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS playlist_reconcile_runs_decision_idx
    ON playlist_reconcile_runs (decision, completed_at DESC, playlist_urn)
    WHERE decision <> 'clean';

CREATE TABLE IF NOT EXISTS playlist_legacy_membership_intents (
    archive_id uuid PRIMARY KEY,
    queue_id uuid UNIQUE,
    source text NOT NULL,
    playlist_urn text NOT NULL,
    legacy_user_id text,
    legacy_desired_revision bigint,
    legacy_synced_revision bigint,
    queue_generation bigint,
    queue_retry_count integer,
    queue_payload jsonb,
    queue_last_error text,
    queue_next_run_at timestamptz,
    queue_failed_at timestamptz,
    queue_created_at timestamptz,
    remote_attempted_generation bigint,
    remote_completed_generation bigint,
    remote_result jsonb,
    local_snapshot_id uuid REFERENCES playlist_remote_snapshots (id) ON DELETE SET NULL,
    classification text NOT NULL DEFAULT 'unclassified',
    archived_at timestamptz NOT NULL DEFAULT now(),
    resolved_at timestamptz,
    CONSTRAINT playlist_legacy_membership_intents_source_valid
        CHECK (source IN ('queue', 'revision')),
    CONSTRAINT playlist_legacy_membership_intents_source_complete
        CHECK ((source = 'queue') = (queue_id IS NOT NULL)),
    CONSTRAINT playlist_legacy_membership_intents_revision_valid
        CHECK (
            source = 'queue'
            OR (
                legacy_desired_revision IS NOT NULL
                AND legacy_synced_revision IS NOT NULL
                AND legacy_desired_revision > legacy_synced_revision
            )
        ),
    CONSTRAINT playlist_legacy_membership_intents_classification_valid
        CHECK (classification IN (
            'unclassified',
            'equal',
            'remote_superset',
            'local_superset',
            'order_only',
            'membership_diverged',
            'resolved',
            'abandoned'
        )),
    CONSTRAINT playlist_legacy_membership_intents_resolution_complete
        CHECK (
            (classification IN ('resolved', 'abandoned')) = (resolved_at IS NOT NULL)
        )
);

CREATE INDEX IF NOT EXISTS playlist_legacy_membership_intents_target_idx
    ON playlist_legacy_membership_intents (playlist_urn, archived_at, archive_id);

CREATE INDEX IF NOT EXISTS playlist_legacy_membership_intents_review_idx
    ON playlist_legacy_membership_intents (archived_at, playlist_urn, archive_id)
    WHERE classification = 'unclassified';

WITH projection_counts AS MATERIALIZED (
    SELECT playlist_urn, count(*)::integer AS track_count
    FROM playlist_track_projection
    GROUP BY playlist_urn
)
INSERT INTO playlist_membership_state (
    playlist_urn,
    projection_revision,
    projection_track_count,
    sync_status,
    next_reconcile_at
)
SELECT playlist.urn,
       greatest(playlist.desired_rev, 0),
       coalesce(projection.track_count, 0),
       CASE
           WHEN playlist.desired_rev > playlist.synced_rev THEN 'legacy_review'
           ELSE 'unhydrated'
       END,
       now()
FROM playlists AS playlist
LEFT JOIN projection_counts AS projection
  ON projection.playlist_urn = playlist.urn
ON CONFLICT (playlist_urn) DO NOTHING;

INSERT INTO playlist_legacy_membership_intents (
    archive_id,
    queue_id,
    source,
    playlist_urn,
    legacy_user_id,
    legacy_desired_revision,
    legacy_synced_revision,
    queue_generation,
    queue_retry_count,
    queue_payload,
    queue_last_error,
    queue_next_run_at,
    queue_failed_at,
    queue_created_at,
    remote_attempted_generation,
    remote_completed_generation,
    remote_result
)
SELECT queue.id,
       queue.id,
       'queue',
       queue.target_urn,
       queue.user_id,
       playlist.desired_rev,
       playlist.synced_rev,
       queue.generation,
       queue.retry_count,
       queue.payload,
       queue.last_error,
       queue.next_run_at,
       queue.failed_at,
       queue.created_at,
       queue.remote_attempted_generation,
       queue.remote_completed_generation,
       queue.remote_result
FROM sync_queue AS queue
LEFT JOIN playlists AS playlist
  ON playlist.urn = queue.target_urn
WHERE queue.action_type = 'playlist_sync'
ON CONFLICT (archive_id) DO NOTHING;

INSERT INTO playlist_legacy_membership_intents (
    archive_id,
    source,
    playlist_urn,
    legacy_user_id,
    legacy_desired_revision,
    legacy_synced_revision
)
SELECT md5('playlist-legacy-revision:' || playlist.urn)::uuid,
       'revision',
       playlist.urn,
       playlist.owner_sc_user_id,
       playlist.desired_rev,
       playlist.synced_rev
FROM playlists AS playlist
WHERE playlist.desired_rev > playlist.synced_rev
  AND NOT EXISTS (
      SELECT 1
      FROM sync_queue AS queue
      WHERE queue.action_type = 'playlist_sync'
        AND queue.target_urn = playlist.urn
  )
ON CONFLICT (archive_id) DO NOTHING;

DELETE FROM sync_queue
WHERE action_type = 'playlist_sync';

DROP INDEX IF EXISTS playlists_pending_sync_idx;

ALTER TABLE playlists
    DROP COLUMN IF EXISTS desired_rev,
    DROP COLUMN IF EXISTS synced_rev,
    DROP COLUMN IF EXISTS tracks_synced_at;
