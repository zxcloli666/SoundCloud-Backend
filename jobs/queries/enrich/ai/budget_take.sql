INSERT INTO ai_resolver_budget (spent_on, spent)
VALUES (current_date, 1)
ON CONFLICT (spent_on) DO UPDATE
    SET spent = ai_resolver_budget.spent + 1
RETURNING spent
