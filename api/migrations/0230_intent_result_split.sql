SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.scd_purge_floor(tbl text, k jsonb)
RETURNS bigint
LANGUAGE sql
STABLE
AS $f$
SELECT coalesce((((max(p.purged_v) >> 4) + 1) << 4) + public.scd_node_ord(), 0)
FROM public.repl_purges p
WHERE p.tbl = scd_purge_floor.tbl
  AND p.pk = scd_purge_floor.k
$f$;
