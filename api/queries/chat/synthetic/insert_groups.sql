INSERT INTO chat.conversations (id, kind, created_by, member_count)
SELECT grp.id, 'group', grp.created_by, grp.member_count
FROM unnest($1::uuid[], $2::text[], $3::integer[]) AS grp(id, created_by, member_count)
ON CONFLICT (id) DO NOTHING
