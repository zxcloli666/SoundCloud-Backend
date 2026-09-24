UPDATE sync_queue
SET locked_at = NULL;

ALTER TABLE sync_queue
    ADD COLUMN generation bigint NOT NULL DEFAULT 1,
    ADD COLUMN lease_id uuid,
    ADD COLUMN lease_generation bigint,
    ADD COLUMN remote_completed_generation bigint,
    ADD COLUMN remote_result jsonb,
    ADD CONSTRAINT sync_queue_generation_positive CHECK (generation > 0),
    ADD CONSTRAINT sync_queue_lease_generation_positive CHECK (
        lease_generation IS NULL OR lease_generation > 0
    ),
    ADD CONSTRAINT sync_queue_remote_generation_positive CHECK (
        remote_completed_generation IS NULL OR remote_completed_generation > 0
    ),
    ADD CONSTRAINT sync_queue_lease_complete CHECK (
        (lease_id IS NULL AND lease_generation IS NULL AND locked_at IS NULL)
        OR
        (lease_id IS NOT NULL AND lease_generation IS NOT NULL AND locked_at IS NOT NULL)
    ),
    ADD CONSTRAINT sync_queue_remote_result_complete CHECK (
        (remote_completed_generation IS NULL AND remote_result IS NULL)
        OR
        (remote_completed_generation IS NOT NULL AND remote_result IS NOT NULL)
    );

UPDATE sync_queue AS current
SET payload = NULL,
    locked_at = NULL,
    lease_id = NULL,
    lease_generation = NULL,
    retry_count = 0,
    last_error = NULL,
    next_run_at = now(),
    created_at = LEAST(current.created_at, legacy.created_at),
    dead = false,
    failed_at = NULL,
    remote_completed_generation = NULL,
    remote_result = NULL,
    generation = GREATEST(current.generation, legacy.generation) + 1
FROM sync_queue AS legacy
WHERE current.action_type = 'playlist_sync'
  AND legacy.action_type = 'playlist_update'
  AND current.user_id = legacy.user_id
  AND current.target_urn = legacy.target_urn;

DELETE FROM sync_queue AS legacy
USING sync_queue AS current
WHERE legacy.action_type = 'playlist_update'
  AND current.action_type = 'playlist_sync'
  AND current.user_id = legacy.user_id
  AND current.target_urn = legacy.target_urn;

UPDATE sync_queue
SET action_type = 'playlist_sync',
    payload = NULL,
    retry_count = 0,
    last_error = NULL,
    next_run_at = now(),
    dead = false,
    failed_at = NULL,
    remote_completed_generation = NULL,
    remote_result = NULL,
    generation = generation + 1
WHERE action_type = 'playlist_update';

DROP INDEX sync_queue_target_uq;

CREATE UNIQUE INDEX sync_queue_target_uq
    ON sync_queue (user_id, action_type, target_urn)
    WHERE action_type <> 'comment';

CREATE INDEX sync_queue_live_lease_idx
    ON sync_queue (user_id, target_urn, locked_at)
    WHERE lease_id IS NOT NULL;

CREATE INDEX sync_queue_head_idx
    ON sync_queue (user_id, target_urn, created_at, id)
    WHERE dead = false;
