if redis.call('SET', KEYS[1], '1', 'NX', 'PX', ARGV[1]) then
  return 0
end

local left = redis.call('PTTL', KEYS[1])
if left < 1 then
  left = 1
end

local time = redis.call('TIME')
local due = time[1] * 1000 + math.floor(time[2] / 1000) + left + ARGV[2]

if redis.call('SET', KEYS[2], due, 'NX', 'PX', left + ARGV[3]) then
  return left + 1
end

return -1
