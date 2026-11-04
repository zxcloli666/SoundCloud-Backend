SELECT id, evidence AS "evidence!", evidence_kek_version AS "evidence_kek_version!"
FROM chat.reports
WHERE evidence IS NOT NULL
  AND evidence_kek_version <> $1
LIMIT $2
