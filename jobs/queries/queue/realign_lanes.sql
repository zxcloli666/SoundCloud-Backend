WITH expected AS (
    SELECT kind, lane
    FROM unnest($1::text[], $2::text[]) AS expected(kind, lane)
), scanned AS MATERIALIZED (
    SELECT id
    FROM background_jobs
    WHERE id > $3
    ORDER BY id
    LIMIT $4
), stale AS (
    SELECT job.id, expected.lane
    FROM scanned
    JOIN background_jobs AS job ON job.id = scanned.id
    JOIN expected ON expected.kind = job.kind
    WHERE job.lane <> expected.lane
      AND job.lease_id IS NULL
    FOR UPDATE OF job SKIP LOCKED
), moved AS (
    UPDATE background_jobs AS job
    SET lane = stale.lane,
        updated_at = now()
    FROM stale
    WHERE job.id = stale.id
      AND job.lease_id IS NULL
    RETURNING job.id
)
SELECT (SELECT id FROM scanned ORDER BY id DESC LIMIT 1) AS last,
       (SELECT count(*) FROM moved) AS "moved!"
