UPDATE oauth_app_token_issuance_reservations
SET released_at = now()
WHERE id = $1
  AND released_at IS NULL
