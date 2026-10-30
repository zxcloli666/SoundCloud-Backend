-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS sync_queue_fresh_idx ON public.sync_queue (id) WHERE exec_generation IS DISTINCT FROM generation AND (exec_generation IS NOT NULL OR intent_v > 0);
