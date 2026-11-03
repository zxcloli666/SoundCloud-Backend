-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS sync_queue_cancelled_idx ON public.sync_queue (user_id, target_urn) WHERE cancelled;
