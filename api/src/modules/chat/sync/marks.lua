local ttl = tonumber(ARGV[1])
local user = ARGV[2]
local changed = {}

for index, key in ipairs(KEYS) do
  local seq = tonumber(ARGV[index + 2])
  local stored = tonumber(redis.call('HGET', key, user) or '0')
  if seq > stored then
    redis.call('HSET', key, user, ARGV[index + 2])
    changed[#changed + 1] = index
  end
  redis.call('EXPIRE', key, ttl)
end

return changed
