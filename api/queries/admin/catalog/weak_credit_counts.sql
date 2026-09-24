SELECT evidence AS "evidence!",
       count(*) AS "count!"
FROM track_artists
WHERE evidence IN (
          'uploader_name',
          'title_heuristic',
          'ai_inference',
          'unattributed'
      )
GROUP BY evidence
ORDER BY evidence
