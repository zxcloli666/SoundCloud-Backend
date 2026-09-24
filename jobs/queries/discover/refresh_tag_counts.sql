WITH counted AS (
    SELECT tag, count(*)::bigint AS artist_count
    FROM (
        SELECT unnest(tags) AS tag
        FROM artists
        WHERE merged_into IS NULL
          AND array_length(tags, 1) > 0
    ) AS expanded
    WHERE trim(tag) <> ''
    GROUP BY tag
), upserted AS (
    INSERT INTO discover_tag_counts (tag, artist_count, refreshed_at)
    SELECT tag, artist_count, now()
    FROM counted
    ON CONFLICT (tag) DO UPDATE
        SET artist_count = EXCLUDED.artist_count,
            refreshed_at = EXCLUDED.refreshed_at
    RETURNING tag
)
DELETE FROM discover_tag_counts
WHERE tag NOT IN (SELECT tag FROM counted)
