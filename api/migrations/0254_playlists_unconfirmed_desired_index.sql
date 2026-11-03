-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS playlists_unconfirmed_desired_idx ON public.playlists (sc_mutation_observation) WHERE sc_desired <> '{}'::jsonb AND NOT sc_write_confirmed AND deleted_at IS NULL;
