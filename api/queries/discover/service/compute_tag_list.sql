SELECT tag AS "tag!", artist_count AS "n!"
FROM discover_tag_counts
ORDER BY artist_count DESC, tag
LIMIT $1
