SELECT f.id, f."type" AS "item_type!", f.sc_urn, f.weight, f.active, f.created_at
FROM featured_items f
WHERE f.active
  AND CASE f."type"
      WHEN 'track' THEN EXISTS (
          SELECT 1 FROM tracks t WHERE t.urn = f.sc_urn AND t.sharing = 'public' AND t.deleted_at IS NULL
      )
      WHEN 'playlist' THEN EXISTS (
          SELECT 1 FROM playlists p WHERE p.urn = f.sc_urn AND p.sharing = 'public' AND p.deleted_at IS NULL
      )
      WHEN 'user' THEN EXISTS (
          SELECT 1 FROM users u WHERE u.urn = f.sc_urn
      )
      ELSE false
  END
ORDER BY -ln(1.0 - random()) / greatest(f.weight, 1), f.id
LIMIT 1
