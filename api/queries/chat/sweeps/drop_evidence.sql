UPDATE chat.reports
SET evidence = NULL,
    evidence_kek_version = NULL,
    purge_after = NULL
WHERE id IN (
    SELECT id
    FROM chat.reports
    WHERE purge_after <= now()
    ORDER BY purge_after
    LIMIT $1
    FOR UPDATE SKIP LOCKED
)
