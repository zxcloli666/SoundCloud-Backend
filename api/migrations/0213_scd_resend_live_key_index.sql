-- no-transaction
CREATE INDEX CONCURRENTLY IF NOT EXISTS repl_resend_live_key ON public.repl_resend ((CASE WHEN tbl = 'repl_tombstones' THEN pk ->> 'tbl' ELSE tbl END), (CASE WHEN tbl = 'repl_tombstones' THEN pk -> 'pk' ELSE pk END), tbl, pk);
