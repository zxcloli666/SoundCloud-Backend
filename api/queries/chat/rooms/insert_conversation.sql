INSERT INTO chat.conversations (id, kind, room_ref, created_by, room_alive_until)
VALUES ($1, 'room', $2, $3, now() + interval '10 minutes')
ON CONFLICT (id) DO NOTHING
