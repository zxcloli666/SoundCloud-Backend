INSERT INTO chat.meta (id, incarnation)
VALUES (1, gen_random_uuid())
ON CONFLICT (id) DO NOTHING
