-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS repl_purges_at_idx ON public.repl_purges (at);
