SET LOCAL lock_timeout = '5s';

ALTER TABLE transcription_wire_state SET (
    fillfactor = 80,
    autovacuum_vacuum_scale_factor = 0,
    autovacuum_vacuum_threshold = 200
);
