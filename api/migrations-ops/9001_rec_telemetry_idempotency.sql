ALTER TABLE rec_impressions
    ADD COLUMN IF NOT EXISTS event_id uuid,
    ADD COLUMN IF NOT EXISTS request_id uuid;

ALTER TABLE rec_hard_negatives
    ADD COLUMN IF NOT EXISTS event_id uuid,
    ALTER COLUMN predicted_score DROP NOT NULL;
