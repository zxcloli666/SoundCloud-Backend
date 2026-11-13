-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS lyrics_repair_due_important_idx ON public.lyrics_repair (priority, next_run_at, sc_track_id) WHERE kind = 'refetch' AND status = 'pending' AND claim_job_id IS NULL;
