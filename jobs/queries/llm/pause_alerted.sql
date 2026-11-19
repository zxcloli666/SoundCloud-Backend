UPDATE llm_pause
SET alerted_at = now(),
    updated_at = now()
WHERE ladder = $1
