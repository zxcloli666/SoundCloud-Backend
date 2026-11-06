if redis.call('ZSCORE', KEYS[1], ARGV[1]) == false then
  return 0
end

redis.call('ZADD', KEYS[1], ARGV[2], ARGV[1])
redis.call('EXPIRE', KEYS[1], ARGV[3])
return 1
