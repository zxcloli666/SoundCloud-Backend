SELECT report.kind,
       report.media_id,
       conversation.kind AS "conversation_kind?"
FROM chat.reports AS report
LEFT JOIN chat.conversations AS conversation ON conversation.id = report.conversation_id
WHERE report.id = $1
