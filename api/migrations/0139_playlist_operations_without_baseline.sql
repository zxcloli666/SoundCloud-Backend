ALTER TABLE playlist_membership_operations
    ALTER COLUMN base_observation_id DROP NOT NULL,
    DROP CONSTRAINT IF EXISTS playlist_membership_operations_base_generation_positive,
    DROP CONSTRAINT IF EXISTS playlist_membership_operations_base_generation_valid,
    ADD CONSTRAINT playlist_membership_operations_base_generation_valid
        CHECK (
            (base_baseline_generation = 0 AND base_observation_id IS NULL)
            OR (base_baseline_generation > 0 AND base_observation_id IS NOT NULL)
        );
