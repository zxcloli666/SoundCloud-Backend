INSERT INTO chat.reports (
    id, reporter, target_user, conversation_id, seq_from, seq_to, reason, comment, weight,
    evidence, evidence_kek_version, kind, media_id, room_public
)
VALUES ($1, $2, $3, $4, 0, 0, $5, NULLIF($6::text, ''), $7, $8, $9, 'backdrop', $10, $11)
