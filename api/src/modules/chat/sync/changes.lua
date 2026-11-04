local version_key = KEYS[1]
local deltas_key = KEYS[2]
local pos = tonumber(ARGV[1])
local ttl = tonumber(ARGV[2])

local stored = redis.call('HMGET', version_key, 'v', 'f', 'rq')
local fresh = '0'
if not stored[1] then
  local clock = redis.call('TIME')
  local now = string.format('%d', tonumber(clock[1]) * 1000 + math.floor(tonumber(clock[2]) / 1000))
  redis.call('HSET', version_key, 'v', now, 'f', now)
  redis.call('DEL', deltas_key)
  stored = { now, now, false }
  fresh = '1'
end
redis.call('EXPIRE', version_key, ttl)
redis.call('EXPIRE', deltas_key, ttl)

local version = stored[1]
local floor = stored[2] or version
local reply = { version, floor, stored[3] or '', fresh }

if fresh == '0' and pos < tonumber(version) and pos >= tonumber(floor) then
  local changed = redis.call('ZRANGEBYSCORE', deltas_key, '(' .. ARGV[1], '+inf', 'WITHSCORES')
  for _, item in ipairs(changed) do
    reply[#reply + 1] = item
  end
end

return reply
