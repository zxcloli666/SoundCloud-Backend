-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS transcription_wire_state_reupload_idx ON transcription_wire_state (sc_track_id) INCLUDE (upload_generation) WHERE status = 'quarantined' AND quarantine_reason IN ('new_upload_during_pending', 'reopen_superseded');
