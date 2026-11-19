INSERT INTO llm_spend (
    ladder, period, sc_track_id, call, provider, model,
    input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, usd
)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11::float8::numeric)
