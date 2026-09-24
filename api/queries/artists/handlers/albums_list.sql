SELECT al.id,
       al.title,
       al.type                                                                                   AS kind,
       al.release_year,
       al.cover_url,
       CASE WHEN al.primary_artist_id = $1 THEN 'primary' ELSE COALESCE(credit.role, 'featured') END AS "role!"
FROM albums al
         LEFT JOIN LATERAL (
    SELECT aa.role
    FROM album_artists aa
    WHERE aa.album_id = al.id
      AND aa.artist_id = $1
    ORDER BY (aa.role = 'primary') DESC, aa.role
    LIMIT 1
    ) AS credit ON true
WHERE al.id IN (SELECT id
                FROM albums
                WHERE primary_artist_id = $1
                UNION
                SELECT album_id
                FROM album_artists
                WHERE artist_id = $1
                UNION
                SELECT wta.album_id
                FROM wanted_track_albums wta
                         JOIN wanted_tracks wt ON wt.id = wta.wanted_track_id
                WHERE wt.primary_artist_id = $1)
ORDER BY COALESCE(al.release_year, 0) DESC, al.title
