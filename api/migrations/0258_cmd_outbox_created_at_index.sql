-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS cmd_outbox_created_at_idx ON public.cmd_outbox (created_at);
