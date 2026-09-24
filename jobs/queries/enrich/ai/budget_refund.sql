UPDATE ai_resolver_budget
SET spent = spent - 1
WHERE spent_on = current_date
  AND spent > 0
