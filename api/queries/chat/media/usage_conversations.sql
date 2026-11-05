WITH own AS (
    SELECT root.origin_conversation,
           root.bytes + coalesce(thumb.bytes, 0) AS bytes
    FROM chat.media AS root
    LEFT JOIN chat.media AS thumb
        ON thumb.parent_id = root.id
       AND thumb.kind = 'thumb'
    WHERE root.owner_user = $1
      AND root.kind <> 'thumb'
      AND root.status IN ('uploaded', 'attached')
),
grouped AS (
    SELECT origin_conversation,
           sum(bytes)::bigint AS bytes,
           count(*)::integer AS files
    FROM own
    GROUP BY origin_conversation
    ORDER BY 2 DESC, 1
    LIMIT $2
)
SELECT grouped.origin_conversation AS "conversation_id!",
       grouped.bytes AS "bytes!",
       grouped.files AS "files!",
       conversation.kind AS "kind?",
       coalesce(conversation.deleted_at IS NULL, false) AS "live!",
       EXISTS (
           SELECT 1
           FROM chat.members AS member
           WHERE member.conversation_id = grouped.origin_conversation
             AND member.user_id = $1
             AND member.state = 'active'
       ) AS "member!"
FROM grouped
LEFT JOIN chat.conversations AS conversation ON conversation.id = grouped.origin_conversation
ORDER BY grouped.bytes DESC, grouped.origin_conversation
