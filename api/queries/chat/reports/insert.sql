INSERT INTO chat.reports (
    id, reporter, target_user, conversation_id, seq_from, seq_to, reason, comment, weight,
    evidence, evidence_kek_version
)
VALUES ($1, $2, $3, $4, $5, $6, $7, NULLIF($8::text, ''), $9, $10, $11)
