CREATE TABLE recommendation_colike_cursor (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    next_a_id uuid NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now()
);
