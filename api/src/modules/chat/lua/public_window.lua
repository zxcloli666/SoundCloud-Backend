if redis.call('SET', KEYS[1], '1', 'NX', 'PX', ARGV[1]) then
  return 0
end

local left = redis.call('PTTL', KEYS[1])
if left < 1 then
  left = 1
end

if redis.call('SET', KEYS[2], '1', 'NX', 'PX', left) then
  return left + 1
end

return -1
