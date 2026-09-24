SELECT generation,
       lease_id,
       lease_generation,
       remote_completed_generation,
       remote_result
FROM sync_queue
WHERE id = $1
FOR UPDATE
