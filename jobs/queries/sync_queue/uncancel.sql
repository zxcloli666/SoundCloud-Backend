UPDATE sync_queue
SET cancelled = false
WHERE id = $1
  AND cancelled
