ALTER TABLE lyrics_cache
    ADD COLUMN IF NOT EXISTS embedding_state varchar(16) DEFAULT 'legacy';

ALTER TABLE lyrics_cache
    ALTER COLUMN embedding_state DROP DEFAULT;

ALTER TABLE lyrics_cache
    ADD CONSTRAINT lyrics_cache_embedding_state_valid CHECK (
        embedding_state IS NULL
        OR embedding_state IN (
            'legacy', 'queued', 'pending', 'dispatched',
            'done', 'skipped', 'quarantined'
        )
    );

CREATE TABLE IF NOT EXISTS lyrics_embedding_wire_state (
    sc_track_id text PRIMARY KEY,
    status varchar(16) NOT NULL,
    lyrics_created_at timestamp,
    request_version smallint,
    request_text text,
    request_language varchar(8),
    request_sha256 bytea,
    request_message_id varchar(128),
    first_publish_attempt_at timestamptz,
    publish_retry_until timestamptz,
    publish_acknowledged_at timestamptz,
    completed_at timestamptz,
    quarantine_reason varchar(64),
    result_consumer varchar(96),
    result_stream varchar(128),
    result_kind varchar(8),
    result_lease_id uuid,
    result_lease_expires_at timestamptz,
    result_stream_sequence bigint,
    result_published_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT lyrics_embedding_wire_state_status_valid CHECK (
        status IN ('pending', 'done', 'skipped', 'quarantined')
    ),
    CONSTRAINT lyrics_embedding_wire_state_request_valid CHECK (
        request_version IS NULL
        OR (
            request_version IS NOT NULL
            AND request_version = 1
            AND lyrics_created_at IS NOT NULL
            AND request_text IS NOT NULL
            AND octet_length(request_text) BETWEEN 1 AND 16000
            AND request_sha256 IS NOT NULL
            AND octet_length(request_sha256) = 32
            AND request_message_id IS NOT NULL
            AND octet_length(request_message_id) BETWEEN 1 AND 128
        )
    ),
    CONSTRAINT lyrics_embedding_wire_state_pending_valid CHECK (
        status <> 'pending'
        OR (
            request_version IS NOT NULL
            AND request_version = 1
            AND first_publish_attempt_at IS NOT NULL
            AND publish_retry_until IS NOT NULL
            AND publish_retry_until > first_publish_attempt_at
            AND completed_at IS NULL
        )
    ),
    CONSTRAINT lyrics_embedding_wire_state_terminal_valid CHECK (
        status = 'pending' OR completed_at IS NOT NULL
    ),
    CONSTRAINT lyrics_embedding_wire_state_quarantine_valid CHECK (
        status <> 'quarantined' OR quarantine_reason IS NOT NULL
    ),
    CONSTRAINT lyrics_embedding_wire_state_result_identity_valid CHECK (
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
            AND result_kind IN ('vector', 'skipped')
            AND result_stream_sequence IS NOT NULL
            AND result_stream_sequence > 0
            AND result_published_at IS NOT NULL
        )
    ),
    CONSTRAINT lyrics_embedding_wire_state_result_lease_valid CHECK (
        (
            result_lease_id IS NULL
            AND result_lease_expires_at IS NULL
        )
        OR (
            result_lease_id IS NOT NULL
            AND result_lease_expires_at IS NOT NULL
            AND status = 'pending'
            AND result_consumer IS NOT NULL
        )
    ),
    CONSTRAINT lyrics_embedding_wire_state_result_terminal_valid CHECK (
        result_kind IS NULL
        OR status NOT IN ('done', 'skipped')
        OR (status = 'done' AND result_kind = 'vector')
        OR (status = 'skipped' AND result_kind = 'skipped')
    )
);

CREATE INDEX IF NOT EXISTS storage_event_transcription_reap_idx
    ON storage_event_state (updated_at, sc_track_id)
    INCLUDE (uploaded_generation)
    WHERE uploaded_generation > 0 AND transcription_generation IS NULL;

CREATE INDEX IF NOT EXISTS lyrics_cache_embedding_reap_idx
    ON lyrics_cache (created_at, sc_track_id)
    WHERE embedded_at IS NULL
      AND embedding_state IS NULL
      AND length(coalesce(plain_text, synced_lrc, '')) > 30;

CREATE INDEX IF NOT EXISTS lyrics_embedding_pending_idx
    ON lyrics_embedding_wire_state (first_publish_attempt_at, sc_track_id)
    WHERE status = 'pending';

CREATE INDEX IF NOT EXISTS transcription_wire_pending_idx
    ON transcription_wire_state (dispatched_at, sc_track_id)
    WHERE status = 'pending';
