SELECT EXISTS (
    SELECT 1 FROM remote_link_codes WHERE code_hash = $1 AND expires_at > now()
) AS "in_use!"
