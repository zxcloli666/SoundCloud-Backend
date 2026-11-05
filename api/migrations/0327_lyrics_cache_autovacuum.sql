SET LOCAL lock_timeout = '5s';

ALTER TABLE lyrics_cache SET (
    autovacuum_vacuum_scale_factor = 0.01,
    autovacuum_vacuum_insert_scale_factor = 0.01
);
