local browser_ua = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36"
local kind = inputs.kind
local id = tonumber(inputs.id)
local page = tonumber(inputs.start_page) or 1
local per_page = tonumber(inputs.per_page) or 50
local max_pages = tonumber(inputs.max_pages) or 1

if id == nil or id <= 0 then
  error("invalid genius entity id")
end
if page < 1 or per_page < 1 or per_page > 50 or max_pages < 1 or max_pages > 20 then
  error("invalid genius pagination")
end
if kind ~= "artist_songs" and kind ~= "artist_albums" and kind ~= "album_tracks" then
  error("invalid genius pagination kind")
end

local items = {}
local complete = false
local next_page = page

local function endpoint(current_page)
  if kind == "artist_songs" then
    return "https://genius.com/api/artists/" .. tostring(id) .. "/songs?per_page=" .. tostring(per_page) .. "&page=" .. tostring(current_page) .. "&sort=popularity"
  end
  if kind == "artist_albums" then
    return "https://genius.com/api/artists/" .. tostring(id) .. "/albums?per_page=" .. tostring(per_page) .. "&page=" .. tostring(current_page)
  end
  return "https://genius.com/api/albums/" .. tostring(id) .. "/tracks?per_page=" .. tostring(per_page) .. "&page=" .. tostring(current_page)
end

local function collection(body)
  if kind == "artist_songs" then
    return body.songs or {}
  end
  if kind == "artist_albums" then
    return body.albums or {}
  end
  return body.tracks or {}
end

for _ = 1, max_pages do
  local response = http({
    url = endpoint(page),
    method = "GET",
    headers = {
      ["Accept-Encoding"] = "identity",
      ["Accept"] = "application/json",
      ["Accept-Language"] = "en-US,en;q=0.9",
      ["User-Agent"] = browser_ua
    }
  })
  if response.status == 404 or response.status == 410 then
    complete = true
    break
  end
  if response.status ~= 200 then
    error("genius pagination status " .. tostring(response.status))
  end
  local payload = json_decode(response.body)
  if type(payload) ~= "table" or type(payload.response) ~= "table" then
    error("invalid genius pagination payload")
  end
  local page_items = collection(payload.response)
  if type(page_items) ~= "table" then
    error("invalid genius pagination collection")
  end
  for _, item in ipairs(page_items) do
    items[#items + 1] = item
  end
  local remote_next = payload.response.next_page
  if kind == "artist_songs" then
    if #page_items < per_page then
      complete = true
      break
    end
    page = page + 1
  elseif remote_next == nil then
    complete = true
    break
  else
    page = tonumber(remote_next)
    if page == nil or page < 1 then
      error("invalid genius next page")
    end
  end
  next_page = page
end

if #items == 0 and complete then
  return {
    kind = "empty",
    items = {},
    complete = true,
    next_page = next_page,
    __verdict = "terminal"
  }
end

return {
  kind = "found",
  items = items,
  complete = complete,
  next_page = next_page
}
