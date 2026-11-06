local due = redis.call('GET', KEYS[1])
if not due then
  return 0
end

local time = redis.call('TIME')
if time[1] * 1000 + math.floor(time[2] / 1000) < tonumber(due) then
  return 0
end

redis.call('DEL', KEYS[1])
return 1
