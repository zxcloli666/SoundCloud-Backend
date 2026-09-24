CREATE TABLE IF NOT EXISTS audio_index_wire_state (
    sc_track_id text PRIMARY KEY,
    status varchar(16) NOT NULL,
    upload_generation bigint,
    dispatched_at timestamptz,
    completed_at timestamptz,
    quarantine_reason varchar(64),
    result_consumer varchar(96),
    result_stream varchar(128),
    result_stream_sequence bigint,
    result_published_at timestamptz,
    result_lease_id uuid,
    result_lease_expires_at timestamptz,
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT audio_index_wire_state_status_valid CHECK (
        status IN ('pending', 'done', 'quarantined')
    ),
    CONSTRAINT audio_index_wire_state_generation_valid CHECK (
        upload_generation IS NULL OR upload_generation > 0
    ),
    CONSTRAINT audio_index_wire_state_pending_valid CHECK (
        status <> 'pending'
        OR (upload_generation IS NOT NULL AND dispatched_at IS NOT NULL)
    ),
    CONSTRAINT audio_index_wire_state_result_sequence_valid CHECK (
        result_stream_sequence IS NULL OR result_stream_sequence > 0
    ),
    CONSTRAINT audio_index_wire_state_lease_valid CHECK (
        (result_lease_id IS NULL) = (result_lease_expires_at IS NULL)
    )
);

INSERT INTO audio_index_wire_state (
    sc_track_id,
    status,
    upload_generation,
    dispatched_at,
    completed_at,
    updated_at
)
SELECT track.sc_track_id,
       'done',
       CASE
           WHEN event.uploaded_generation > 0 THEN event.uploaded_generation
       END,
       track.indexed_at,
       track.indexed_at,
       now()
FROM tracks AS track
LEFT JOIN storage_event_state AS event
  ON event.sc_track_id = track.sc_track_id
WHERE track.index_state = 'indexed'
ON CONFLICT (sc_track_id) DO NOTHING;
