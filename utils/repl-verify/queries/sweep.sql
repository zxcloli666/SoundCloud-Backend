SELECT s.state AS "state!", s.sent AS "sent!", s.skipped AS "skipped!", s.gone AS "gone!", s.queued AS "queued!"
FROM public.scd_sweep($1) AS s
