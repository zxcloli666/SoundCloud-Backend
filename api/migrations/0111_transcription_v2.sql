ALTER TABLE transcription_wire_state
    ADD COLUMN IF NOT EXISTS attempt bigint NOT NULL DEFAULT 1,
    ADD COLUMN IF NOT EXISTS reopen_count integer NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS result_rank smallint,
    ADD COLUMN IF NOT EXISTS reason varchar(32),
    ADD COLUMN IF NOT EXISTS sync_version varchar(128),
    ADD COLUMN IF NOT EXISTS reopened_for_sync_version varchar(128),
    ADD COLUMN IF NOT EXISTS confidence double precision,
    ADD COLUMN IF NOT EXISTS placed_share double precision,
    ADD COLUMN IF NOT EXISTS aligned_share double precision,
    ADD COLUMN IF NOT EXISTS lines_total integer,
    ADD COLUMN IF NOT EXISTS lines_unplaced integer,
    ADD COLUMN IF NOT EXISTS result_language varchar(8);

ALTER TABLE transcription_wire_state
    DROP CONSTRAINT IF EXISTS transcription_wire_state_status_valid;

ALTER TABLE transcription_wire_state
    ADD CONSTRAINT transcription_wire_state_status_valid CHECK (
        status IN ('pending', 'done', 'empty', 'rejected', 'reopenable', 'quarantined')
    );

ALTER TABLE transcription_wire_state
    ADD CONSTRAINT transcription_wire_state_attempt_valid CHECK (
        attempt > 0 AND reopen_count >= 0
    );

ALTER TABLE transcription_wire_state
    ADD CONSTRAINT transcription_wire_state_rank_valid CHECK (
        result_rank IS NULL OR result_rank BETWEEN 1 AND 4
    );

ALTER TABLE transcription_wire_state
    ADD CONSTRAINT transcription_wire_state_rejected_valid CHECK (
        status <> 'rejected'
        OR (reason IS NOT NULL AND sync_version IS NOT NULL AND completed_at IS NOT NULL)
    );

ALTER TABLE transcription_wire_state
    ADD CONSTRAINT transcription_wire_state_reopenable_valid CHECK (
        status <> 'reopenable'
        OR (reason IS NOT NULL AND completed_at IS NOT NULL AND upload_generation IS NOT NULL)
    );

CREATE INDEX IF NOT EXISTS transcription_wire_reopen_idx
    ON transcription_wire_state (completed_at, sc_track_id)
    WHERE status IN ('rejected', 'reopenable');

CREATE TABLE IF NOT EXISTS transcription_sync_versions (
    sync_version varchar(128) PRIMARY KEY,
    first_seen_at timestamptz NOT NULL DEFAULT now(),
    last_seen_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT transcription_sync_versions_value_valid CHECK (
        octet_length(sync_version) BETWEEN 1 AND 128
    )
);

CREATE INDEX IF NOT EXISTS transcription_sync_versions_newest_idx
    ON transcription_sync_versions (first_seen_at DESC, sync_version DESC);

ALTER TABLE lyrics_embedding_wire_state
    ADD COLUMN IF NOT EXISTS lyrics_content_generation bigint,
    ADD COLUMN IF NOT EXISTS result_reason varchar(32),
    ADD COLUMN IF NOT EXISTS reopen_count integer NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS result_rank smallint;

ALTER TABLE lyrics_embedding_wire_state
    ADD CONSTRAINT lyrics_embedding_wire_state_rank_valid CHECK (
        result_rank IS NULL OR result_rank BETWEEN 1 AND 4
    );

ALTER TABLE lyrics_embedding_wire_state
    DROP CONSTRAINT IF EXISTS lyrics_embedding_wire_state_status_valid;

ALTER TABLE lyrics_embedding_wire_state
    ADD CONSTRAINT lyrics_embedding_wire_state_status_valid CHECK (
        status IN ('pending', 'done', 'skipped', 'failed', 'reopenable', 'quarantined')
    );

ALTER TABLE lyrics_embedding_wire_state
    DROP CONSTRAINT IF EXISTS lyrics_embedding_wire_state_result_identity_valid;

ALTER TABLE lyrics_embedding_wire_state
    ADD CONSTRAINT lyrics_embedding_wire_state_result_identity_valid CHECK (
        (
            result_consumer IS NULL
            AND result_stream IS NULL
            AND result_kind IS NULL
            AND result_stream_sequence IS NULL
            AND result_published_at IS NULL
        )
        OR (
            result_consumer IS NOT NULL
            AND octet_length(result_consumer) BETWEEN 1 AND 96
            AND result_stream IS NOT NULL
            AND octet_length(result_stream) BETWEEN 1 AND 128
            AND result_kind IS NOT NULL
            AND result_kind IN ('vector', 'skipped', 'failed', 'reopen')
            AND result_stream_sequence IS NOT NULL
            AND result_stream_sequence > 0
            AND result_published_at IS NOT NULL
        )
    );

ALTER TABLE lyrics_embedding_wire_state
    DROP CONSTRAINT IF EXISTS lyrics_embedding_wire_state_result_terminal_valid;

ALTER TABLE lyrics_embedding_wire_state
    ADD CONSTRAINT lyrics_embedding_wire_state_result_terminal_valid CHECK (
        result_kind IS NULL
        OR status NOT IN ('done', 'skipped', 'failed', 'reopenable')
        OR (status = 'done' AND result_kind = 'vector')
        OR (status = 'skipped' AND result_kind = 'skipped')
        OR (status = 'failed' AND result_kind = 'failed')
        OR (status = 'reopenable' AND result_kind = 'reopen')
    );

ALTER TABLE lyrics_embedding_wire_state
    ADD CONSTRAINT lyrics_embedding_wire_state_reopen_count_valid CHECK (
        reopen_count >= 0
    );

ALTER TABLE lyrics_cache
    DROP CONSTRAINT IF EXISTS lyrics_cache_embedding_state_valid;

ALTER TABLE lyrics_cache
    ADD CONSTRAINT lyrics_cache_embedding_state_valid CHECK (
        embedding_state IS NULL
        OR embedding_state IN (
            'legacy', 'queued', 'pending', 'dispatched',
            'done', 'skipped', 'failed', 'quarantined'
        )
    ) NOT VALID;

ALTER TABLE lyrics_cache
    VALIDATE CONSTRAINT lyrics_cache_embedding_state_valid;
