UPDATE featured_items
SET "type" = COALESCE($2, "type"),
    sc_urn = COALESCE($3, sc_urn),
    weight = COALESCE($4, weight),
    active = COALESCE($5, active)
WHERE id = $1
RETURNING id, "type" AS "item_type!", sc_urn, weight, active, created_at
