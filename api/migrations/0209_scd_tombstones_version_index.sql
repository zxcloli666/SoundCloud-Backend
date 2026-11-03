-- no-transaction
CREATE UNIQUE INDEX CONCURRENTLY IF NOT EXISTS repl_tombstones_version_key ON public.repl_tombstones (tbl, pk, v);
