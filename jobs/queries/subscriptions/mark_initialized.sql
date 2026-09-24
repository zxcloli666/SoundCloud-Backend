INSERT INTO subscription_snapshot_state (singleton)
VALUES (true)
ON CONFLICT (singleton) DO NOTHING
