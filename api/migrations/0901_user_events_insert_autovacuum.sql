SET LOCAL lock_timeout = '5s';

ALTER TABLE user_events SET (
    autovacuum_vacuum_insert_scale_factor = 0,
    autovacuum_vacuum_insert_threshold = 50000
);
