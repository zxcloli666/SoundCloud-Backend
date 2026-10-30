SELECT page.last_id,
       page.added AS "added!"
FROM public.scd_job_status_backfill($1, $2) AS page
