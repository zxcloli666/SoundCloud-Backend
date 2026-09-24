INSERT INTO rec_impressions (
    event_id,
    request_id,
    sc_user_id,
    sc_track_id,
    cluster_id,
    source,
    position,
    score,
    features,
    shown_at
)
SELECT event_id,
       request_id,
       sc_user_id,
       sc_track_id,
       cluster_id,
       source,
       position,
       CASE WHEN score_present THEN score END,
       NULLIF(features, 'null'::jsonb),
       shown_at
FROM UNNEST(
    $1::uuid[],
    $2::uuid[],
    $3::text[],
    $4::text[],
    $5::text[],
    $6::varchar[],
    $7::int2[],
    $8::bool[],
    $9::float4[],
    $10::jsonb[],
    $11::timestamptz[]
) AS item(
    event_id,
    request_id,
    sc_user_id,
    sc_track_id,
    cluster_id,
    source,
    position,
    score_present,
    score,
    features,
    shown_at
)
ON CONFLICT (event_id) WHERE event_id IS NOT NULL DO NOTHING
