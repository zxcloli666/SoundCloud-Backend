local ttl = tonumber(ARGV[1])
local kept = tonumber(ARGV[2])
local seq = tonumber(ARGV[3])
local first = 1

if seq > 0 then
  first = 2
  local stored = tonumber(redis.call('GET', KEYS[1]) or '0')
  if seq > stored then
    redis.call('SET', KEYS[1], string.format('%d', seq), 'EX', ttl)
  else
    redis.call('EXPIRE', KEYS[1], ttl)
  end
end

local clock = redis.call('TIME')
local now = tonumber(clock[1]) * 1000 + math.floor(tonumber(clock[2]) / 1000)

for index = first, #KEYS, 2 do
  local version_key = KEYS[index]
  local deltas_key = KEYS[index + 1]
  local stored = redis.call('HMGET', version_key, 'v', 'f')
  local version = tonumber(stored[1])
  local floor = tonumber(stored[2])
  if not version then
    version = now
    floor = now
  end
  floor = floor or version
  version = math.max(version + 1, now)
  local score = string.format('%d', version)
  for member = 4, #ARGV do
    redis.call('ZADD', deltas_key, score, ARGV[member])
  end
  local excess = redis.call('ZCARD', deltas_key) - kept
  if excess > 0 then
    local dropped = redis.call('ZRANGE', deltas_key, excess - 1, excess - 1, 'WITHSCORES')
    floor = math.max(floor, tonumber(dropped[2]))
    redis.call('ZREMRANGEBYRANK', deltas_key, 0, excess - 1)
  end
  redis.call('HSET', version_key, 'v', score, 'f', string.format('%d', floor))
  redis.call('EXPIRE', version_key, ttl)
  redis.call('EXPIRE', deltas_key, ttl)
end

return 1
