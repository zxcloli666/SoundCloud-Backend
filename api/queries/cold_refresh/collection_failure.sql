SELECT split_part(f.last_error, ':', 1) AS "code!",
       CASE WHEN split_part(f.last_error, ':', 1) = 'auth_required' THEN NULL
            ELSE GREATEST(0, ceil(extract(epoch FROM f.failed_at + interval '6 hours' - now())))::bigint END AS hold_seconds
FROM background_job_failures f
WHERE f.kind = 'catalog.collection' AND f.dedup_key = $1
  AND f.failed_at > COALESCE($2::timestamptz, '-infinity')
  AND split_part(f.last_error, ':', 1) IN ('auth_required', 'not_found', 'forbidden')
  AND NOT (split_part(f.last_error, ':', 1) = 'auth_required' AND EXISTS (
      SELECT 1 FROM soundcloud_connections c WHERE c.soundcloud_user_id = ANY($3) AND c.created_at > f.failed_at))
ORDER BY f.failed_at DESC
LIMIT 1
