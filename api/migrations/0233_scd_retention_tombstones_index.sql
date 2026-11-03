-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS repl_tombstones_at_idx ON public.repl_tombstones (at);
