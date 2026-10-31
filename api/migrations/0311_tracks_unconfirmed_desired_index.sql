-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS tracks_unconfirmed_desired_idx ON public.tracks (sc_mutation_observation) WHERE sc_desired <> '{}'::jsonb AND NOT sc_write_confirmed AND deleted_at IS NULL;
