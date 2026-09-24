CREATE TABLE IF NOT EXISTS pipeline_event_receipts (
    consumer varchar(96) NOT NULL,
    stream varchar(128) NOT NULL,
    stream_sequence bigint NOT NULL,
    event_published_at timestamptz NOT NULL,
    processed_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (consumer, stream, stream_sequence, event_published_at),
    CONSTRAINT pipeline_event_receipts_consumer_valid CHECK (
        octet_length(consumer) BETWEEN 1 AND 96
    ),
    CONSTRAINT pipeline_event_receipts_stream_valid CHECK (
        octet_length(stream) BETWEEN 1 AND 128
    ),
    CONSTRAINT pipeline_event_receipts_sequence_positive CHECK (stream_sequence > 0)
);

CREATE INDEX IF NOT EXISTS pipeline_event_receipts_processed_idx
    ON pipeline_event_receipts (processed_at);

CREATE TABLE IF NOT EXISTS storage_event_state (
    sc_track_id text PRIMARY KEY REFERENCES tracks(sc_track_id) ON DELETE CASCADE,
    stream varchar(128) NOT NULL,
    stream_sequence bigint NOT NULL,
    event_published_at timestamptz NOT NULL,
    uploaded_generation bigint NOT NULL DEFAULT 0,
    transcription_generation bigint,
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT storage_event_state_stream_valid CHECK (
        octet_length(stream) BETWEEN 1 AND 128
    ),
    CONSTRAINT storage_event_state_sequence_positive CHECK (stream_sequence > 0),
    CONSTRAINT storage_event_state_generation_nonnegative CHECK (uploaded_generation >= 0),
    CONSTRAINT storage_event_state_transcription_generation_valid CHECK (
        transcription_generation IS NULL
        OR transcription_generation BETWEEN 0 AND uploaded_generation
    )
);
