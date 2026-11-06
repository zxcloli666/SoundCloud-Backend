DELETE FROM chat.public_mutes
WHERE until < now()
