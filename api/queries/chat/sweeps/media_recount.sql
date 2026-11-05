WITH counted AS (
    SELECT usage.user_id,
           usage.bytes AS was_bytes,
           usage.reserved AS was_reserved,
           usage.files AS was_files,
           coalesce(sum(root.bytes + coalesce(thumb.bytes, 0)) FILTER (WHERE root.status IN ('uploaded', 'attached')), 0)::bigint AS bytes,
           coalesce(sum(root.bytes + coalesce(thumb.bytes, 0)) FILTER (WHERE root.status = 'reserved'), 0)::bigint AS reserved,
           count(root.id) FILTER (WHERE root.status IN ('uploaded', 'attached'))::integer AS files
    FROM chat.usage AS usage
    LEFT JOIN chat.media AS root
      ON root.owner_user = usage.user_id
     AND root.kind <> 'thumb'
    LEFT JOIN chat.media AS thumb
      ON thumb.parent_id = root.id
    WHERE usage.user_id = ANY($1::text[])
    GROUP BY usage.user_id, usage.bytes, usage.reserved, usage.files
),
fixed AS (
    UPDATE chat.usage AS usage
    SET bytes = counted.bytes,
        reserved = counted.reserved,
        files = counted.files,
        updated_at = now()
    FROM counted
    WHERE usage.user_id = counted.user_id
      AND (counted.was_bytes, counted.was_reserved, counted.was_files)
          IS DISTINCT FROM (counted.bytes, counted.reserved, counted.files)
    RETURNING usage.user_id
)
SELECT coalesce(sum(abs(was_bytes - bytes) + abs(was_reserved - reserved)), 0)::bigint AS "drift!"
FROM counted
