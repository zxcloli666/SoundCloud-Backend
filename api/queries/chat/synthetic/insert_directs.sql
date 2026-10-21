INSERT INTO chat.conversations (id, kind, dm_key, created_by, member_count)
SELECT direct.id, 'direct', direct.dm_key, direct.created_by, 2
FROM unnest($1::uuid[], $2::text[], $3::text[]) AS direct(id, dm_key, created_by)
ON CONFLICT DO NOTHING
