CREATE TABLE IF NOT EXISTS transcription_wire_state (
    sc_track_id text PRIMARY KEY,
    status varchar(16) NOT NULL,
    upload_generation bigint,
    dispatched_at timestamptz,
    completed_at timestamptz,
    quarantine_reason varchar(64),
    result_stream_sequence bigint,
    result_published_at timestamptz,
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT transcription_wire_state_status_valid CHECK (
        status IN ('pending', 'done', 'empty', 'quarantined')
    ),
    CONSTRAINT transcription_wire_state_generation_valid CHECK (
        upload_generation IS NULL OR upload_generation > 0
    ),
    CONSTRAINT transcription_wire_state_pending_valid CHECK (
        status <> 'pending'
        OR (upload_generation IS NOT NULL AND dispatched_at IS NOT NULL)
    ),
    CONSTRAINT transcription_wire_state_result_sequence_valid CHECK (
        result_stream_sequence IS NULL OR result_stream_sequence > 0
    )
);

INSERT INTO transcription_wire_state (
    sc_track_id,
    status,
    upload_generation,
    dispatched_at,
    completed_at,
    quarantine_reason,
    updated_at
)
SELECT track.sc_track_id,
       CASE track.transcribe_state
           WHEN 'done' THEN 'done'
           WHEN 'disabled' THEN 'empty'
           ELSE 'quarantined'
       END,
       CASE
           WHEN event.transcription_generation > 0
               THEN event.transcription_generation
           ELSE NULL
       END,
       track.transcribe_at,
       CASE
           WHEN track.transcribe_state IN ('done', 'disabled')
               THEN track.transcribe_at
           ELSE NULL
       END,
       CASE
           WHEN track.transcribe_state = 'pending'
               THEN 'legacy_pending_at_cutover'
           ELSE NULL
       END,
       now()
FROM tracks AS track
LEFT JOIN storage_event_state AS event
  ON event.sc_track_id = track.sc_track_id
WHERE track.transcribe_state IN ('pending', 'done', 'disabled')
ON CONFLICT (sc_track_id) DO NOTHING;

UPDATE tracks
SET transcribe_state = 'quarantined',
    transcribe_at = now(),
    updated_at = now()
WHERE transcribe_state = 'pending';
