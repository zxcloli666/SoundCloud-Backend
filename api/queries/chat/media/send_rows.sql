SELECT media.id,
       media.kind,
       media.status,
       media.owner_user = $2 AS "own!",
       media.origin_conversation,
       coalesce(origin.kind = 'room', false) AS "origin_room!",
       origin.id IS NOT NULL AS "origin_known!",
       EXISTS (
           SELECT 1
           FROM chat.media_refs AS ref
           WHERE ref.media_id = media.id
             AND ref.conversation_id = $3
             AND (ref.seq = 0 OR ref.seq > $4::bigint)
       ) AS "here!",
       EXISTS (
           SELECT 1
           FROM chat.media_refs AS ref
           JOIN chat.conversations AS conversation
               ON conversation.id = ref.conversation_id
              AND conversation.deleted_at IS NULL
           JOIN chat.members AS member
               ON member.conversation_id = ref.conversation_id
              AND member.user_id = $2
              AND member.state IN ('active', 'invited')
           WHERE ref.media_id = media.id
             AND (ref.seq = 0 OR ref.seq > greatest(
                 member.hidden_before_seq,
                 CASE
                     WHEN conversation.kind = 'group'
                         AND (conversation.settings ->> 'history_visible') = 'false' THEN member.joined_seq
                     ELSE 0
                 END
             ))
       ) AS "elsewhere!"
FROM chat.media AS media
LEFT JOIN chat.conversations AS origin ON origin.id = media.origin_conversation
WHERE media.id = ANY($1::uuid[])
