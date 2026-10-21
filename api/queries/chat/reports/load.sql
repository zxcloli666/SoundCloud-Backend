SELECT id,
       reporter,
       target_user,
       conversation_id,
       seq_from,
       seq_to,
       reason,
       comment,
       weight,
       evidence,
       evidence_kek_version,
       state,
       created_at,
       handled_by,
       handled_at
FROM chat.reports
WHERE id = $1
