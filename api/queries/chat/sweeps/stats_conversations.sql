WITH alive AS (
    SELECT kind,
           count(*) AS chats,
           count(*) FILTER (WHERE last_at > now() - interval '1 day') AS active_day,
           count(*) FILTER (WHERE last_at > now() - interval '7 days') AS active_week,
           sum(member_count) AS members
    FROM chat.conversations
    WHERE deleted_at IS NULL
    GROUP BY kind
)
SELECT kinds.kind AS "kind!",
       coalesce(alive.chats, 0) AS "chats!",
       coalesce(alive.active_day, 0) AS "active_day!",
       coalesce(alive.active_week, 0) AS "active_week!",
       coalesce(alive.members, 0)::bigint AS "members!"
FROM unnest($1::text[]) AS kinds (kind)
LEFT JOIN alive ON alive.kind = kinds.kind
