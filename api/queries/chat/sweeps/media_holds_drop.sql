DELETE FROM chat.media_holds AS hold
USING (
    SELECT media_id, report_id
    FROM chat.media_holds AS candidate
    WHERE candidate.until <= now()
       OR NOT EXISTS (
           SELECT 1
           FROM chat.reports AS report
           WHERE report.id = candidate.report_id
             AND report.state = 'open'
       )
    LIMIT $1
    FOR UPDATE SKIP LOCKED
) AS due
WHERE hold.media_id = due.media_id
  AND hold.report_id = due.report_id
RETURNING hold.media_id
