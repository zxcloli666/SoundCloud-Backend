DELETE
FROM admin_catalog_requests
WHERE created_at < now() - interval '7 days'
