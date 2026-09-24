ALTER TABLE oauth_app_tokens
    ADD COLUMN generation uuid;

UPDATE oauth_app_tokens
SET generation = oauth_app_id;

ALTER TABLE oauth_app_tokens
    ALTER COLUMN generation SET NOT NULL,
    ALTER COLUMN generation SET DEFAULT gen_random_uuid();
