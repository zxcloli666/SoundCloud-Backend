SELECT r.id AS "id!", r.name AS "name!", r.avatar_url AS "avatar_url?"
FROM ((SELECT a.id, a.name, a.avatar_url, ac.weight
       FROM artist_coplay ac
                JOIN artists a ON a.id = ac.b_id
       WHERE ac.a_id = $1
         AND a.merged_into IS NULL
       ORDER BY ac.weight DESC
       LIMIT $2)
      UNION ALL
      (SELECT a.id, a.name, a.avatar_url, ac.weight
       FROM artist_coplay ac
                JOIN artists a ON a.id = ac.a_id
       WHERE ac.b_id = $1
         AND a.merged_into IS NULL
       ORDER BY ac.weight DESC
       LIMIT $2)) r
ORDER BY r.weight DESC
LIMIT $2
