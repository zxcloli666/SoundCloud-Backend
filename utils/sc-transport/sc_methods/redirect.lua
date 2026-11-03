local url = inputs.url
if type(url) ~= "string" then
  return { ok = false, reason = "invalid" }
end
if string.sub(url, 1, 7) == "http://" then
  url = "https://" .. string.sub(url, 8)
end
if string.sub(url, 1, 8) ~= "https://" then
  return { ok = false, reason = "invalid" }
end

local resp = http({ url = url, method = "GET" })
local s = resp.status
local location = resp.headers and resp.headers["location"]

if s == 404 or s == 410 then
  return { ok = false, reason = "gone" }
end
if (s == 301 or s == 302 or s == 303 or s == 307 or s == 308) and type(location) == "string" then
  local host = string.match(location, "^https://([^/?#]+)")
  if host ~= nil and (host == "soundcloud.com" or string.sub(host, -15) == ".soundcloud.com") then
    return { ok = true, location = location }
  end
  error("redirect leaves soundcloud")
end
error("redirect status " .. tostring(s))
