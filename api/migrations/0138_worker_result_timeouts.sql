SET LOCAL lock_timeout = '5s';

ALTER TABLE lyrics_embedding_wire_state
    ADD COLUMN IF NOT EXISTS result_timeouts integer NOT NULL DEFAULT 0;

ALTER TABLE transcription_wire_state
    ADD COLUMN IF NOT EXISTS result_timeouts integer NOT NULL DEFAULT 0;
