UPDATE chat.reports
SET evidence = $2,
    evidence_kek_version = $3
WHERE id = $1
  AND evidence_kek_version = $4
