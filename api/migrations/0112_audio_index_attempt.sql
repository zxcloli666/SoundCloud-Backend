ALTER TABLE audio_index_wire_state
    ADD COLUMN attempt integer NOT NULL DEFAULT 1,
    ADD COLUMN outcome_rank smallint NOT NULL DEFAULT 0,
    ADD COLUMN outcome_status varchar(16),
    ADD COLUMN outcome_reason varchar(48);

UPDATE audio_index_wire_state
SET outcome_rank = 4,
    outcome_status = 'ok'
WHERE status = 'done';

ALTER TABLE audio_index_wire_state
    DROP CONSTRAINT audio_index_wire_state_status_valid,
    ADD CONSTRAINT audio_index_wire_state_status_valid CHECK (
        status IN ('pending', 'done', 'terminal', 'reopenable', 'quarantined')
    ),
    ADD CONSTRAINT audio_index_wire_state_attempt_valid CHECK (attempt > 0),
    ADD CONSTRAINT audio_index_wire_state_outcome_rank_valid CHECK (
        outcome_rank BETWEEN 0 AND 4
    ),
    ADD CONSTRAINT audio_index_wire_state_outcome_status_valid CHECK (
        outcome_status IS NULL
        OR outcome_status IN ('ok', 'empty', 'missing', 'rejected', 'failed')
    ),
    ADD CONSTRAINT audio_index_wire_state_outcome_valid CHECK (
        status NOT IN ('terminal', 'reopenable')
        OR (outcome_status IS NOT NULL AND outcome_reason IS NOT NULL)
    );

CREATE INDEX IF NOT EXISTS audio_index_wire_state_reopen_idx
    ON audio_index_wire_state (updated_at, sc_track_id)
    WHERE status = 'reopenable'
       OR (status = 'terminal' AND outcome_reason = 'audio_forbidden');
