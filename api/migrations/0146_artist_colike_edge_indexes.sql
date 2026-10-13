SET LOCAL lock_timeout = '5s';
CREATE INDEX IF NOT EXISTS artist_colike_a_edges_idx ON artist_colike (a_id, w DESC) INCLUDE (b_id);
CREATE INDEX IF NOT EXISTS artist_colike_b_edges_idx ON artist_colike (b_id, w DESC) INCLUDE (a_id);
DROP INDEX IF EXISTS artist_colike_a_w_idx;
DROP INDEX IF EXISTS artist_colike_b_w_idx;
ALTER TABLE artist_colike SET (
    autovacuum_vacuum_scale_factor = 0.01,
    autovacuum_vacuum_insert_scale_factor = 0.01
);
