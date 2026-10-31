-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS cmd_done_at_idx ON public.cmd_done (at);
