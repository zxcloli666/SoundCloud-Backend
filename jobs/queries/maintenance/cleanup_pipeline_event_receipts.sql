WITH expired AS (
    SELECT receipt.consumer,
           receipt.stream,
           receipt.stream_sequence,
           receipt.event_published_at
    FROM pipeline_event_receipts AS receipt
    WHERE receipt.processed_at < now() - interval '7 days'
    ORDER BY receipt.processed_at
    FOR UPDATE OF receipt SKIP LOCKED
    LIMIT 10_000
)
DELETE FROM pipeline_event_receipts AS receipt
USING expired
WHERE receipt.consumer = expired.consumer
  AND receipt.stream = expired.stream
  AND receipt.stream_sequence = expired.stream_sequence
  AND receipt.event_published_at = expired.event_published_at
