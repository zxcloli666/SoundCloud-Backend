INSERT INTO chat.usage AS usage (user_id, reserved, day_bytes, day)
SELECT $1::text, $2::bigint, $2::bigint, current_date
WHERE $2::bigint <= $3::bigint
  AND $2::bigint <= $4::bigint
ON CONFLICT (user_id) DO UPDATE
SET reserved = usage.reserved + EXCLUDED.reserved,
    day_bytes = CASE WHEN usage.day = current_date THEN usage.day_bytes ELSE 0 END + EXCLUDED.reserved,
    day = current_date,
    updated_at = now()
WHERE usage.bytes + usage.reserved + EXCLUDED.reserved <= $3::bigint
  AND CASE WHEN usage.day = current_date THEN usage.day_bytes ELSE 0 END + EXCLUDED.reserved <= $4::bigint
RETURNING bytes, reserved
