WITH repair AS MATERIALIZED (
    SELECT repair.sc_track_id,
           repair.cache_created_at
    FROM lyrics_repair AS repair
    JOIN background_jobs AS job
      ON job.id = $1
     AND job.generation = $2
     AND job.lease_generation = $2
     AND job.lease_id = $3
     AND job.lease_expires_at > now()
    WHERE repair.sc_track_id = $4
      AND repair.kind = 'refetch'
      AND repair.status = 'pending'
      AND repair.claim_job_id = $1
      AND repair.claim_job_generation = $2
      AND repair.claim_job_lease_id = $3
      AND repair.claim_expires_at > now()
    FOR UPDATE OF repair
), locked_track AS MATERIALIZED (
    SELECT track.id
    FROM tracks AS track
    JOIN repair ON repair.sc_track_id = track.sc_track_id
    FOR UPDATE OF track
), cache AS MATERIALIZED (
    SELECT cache.plain_text,
           cache.content_generation,
           (NULLIF(btrim(cache.synced_lrc), '') IS NOT NULL) AS has_synced,
           (cache.source = 'genius'
               AND COALESCE(cache.plain_source, cache.source) = 'genius'
               AND cache.created_at = repair.cache_created_at) AS same_row
    FROM lyrics_cache AS cache
    JOIN repair ON repair.sc_track_id = cache.sc_track_id
    WHERE (SELECT count(*) FROM locked_track) >= 0
    FOR UPDATE OF cache
)
SELECT cache.plain_text,
       cache.content_generation AS "content_generation?",
       COALESCE(cache.same_row, false) AS "same_row!",
       COALESCE(cache.has_synced, false) AS "has_synced!"
FROM repair
         LEFT JOIN cache ON true
