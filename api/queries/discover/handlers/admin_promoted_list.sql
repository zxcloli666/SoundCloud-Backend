SELECT p.id,
       p.entity_type,
       p.entity_id,
       p.position,
       p.active,
       p.note,
       p.created_at,
       p.updated_at,
       COALESCE(a.name, al.title)           AS name,
       COALESCE(a.avatar_url, al.cover_url)  AS image_url
FROM discover_promoted p
LEFT JOIN artists a ON p.entity_type = 'artist' AND a.id = p.entity_id
LEFT JOIN albums al ON p.entity_type = 'album' AND al.id = p.entity_id
ORDER BY p.active DESC, p.position ASC, p.created_at ASC
