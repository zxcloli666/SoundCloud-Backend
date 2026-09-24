UPDATE playlist_membership_operations AS operation
SET outcome = resolution.outcome,
    conflict_code = resolution.conflict_code,
    resolved_at = clock_timestamp()
FROM unnest($2::uuid[], $3::text[], $4::text[])
    AS resolution(operation_id, outcome, conflict_code)
WHERE operation.playlist_urn = $1
  AND operation.operation_id = resolution.operation_id
  AND operation.resolved_at IS NULL
