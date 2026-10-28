local MAX_REQUESTS = 5
local requests = 0

local function spare(count)
  return requests + count <= MAX_REQUESTS
end

local function request(url)
  requests = requests + 1
  local headers = {
    ["Accept-Encoding"] = "identity",
    ["Accept"] = "application/json,text/html;q=0.9,*/*;q=0.8",
    ["Accept-Language"] = "en-US,en;q=0.9"
  }
  if string.find(url, "lrclib.net", 1, true) ~= nil then
    headers["Lrclib-Client"] = "SoundCloud Desktop (https://github.com/zxcloli666/SoundCloud-Desktop)"
  end
  local response = http({ url = url, method = "GET", headers = headers })
  if response.status == 200 then
    return response.body
  end
  if response.status == 404 or response.status == 410 then
    return nil
  end
  error("lyrics upstream status " .. tostring(response.status))
end

local function request_json(url)
  local body = request(url)
  if body == nil then
    return nil
  end
  return json_decode(body)
end

local function nonempty(value)
  return type(value) == "string" and string.find(value, "%S") ~= nil
end

local function decode_html(value)
  value = string.gsub(value, "<br%s*/?>", "\n")
  value = string.gsub(value, "<[^>]+>", "")
  value = string.gsub(value, "&nbsp;", " ")
  value = string.gsub(value, "&amp;", "&")
  value = string.gsub(value, "&lt;", "<")
  value = string.gsub(value, "&gt;", ">")
  value = string.gsub(value, "&#x27;", "'")
  value = string.gsub(value, "&apos;", "'")
  value = string.gsub(value, "&quot;", "\"")
  value = string.gsub(value, "^%s+", "")
  value = string.gsub(value, "%s+$", "")
  return value
end

local function genius_lyrics(url)
  if not nonempty(url) then
    return nil
  end
  if string.sub(url, 1, 19) ~= "https://genius.com/" and string.sub(url, 1, 23) ~= "https://www.genius.com/" then
    error("invalid genius url")
  end
  local html = request(url)
  if html == nil then
    return nil
  end
  local parts = {}
  for container in string.gmatch(html, '<div[^>]-data%-lyrics%-container="true"[^>]*>(.-)</div>') do
    local text = decode_html(container)
    if nonempty(text) and #text > 20 then
      parts[#parts + 1] = text
    end
  end
  if #parts > 0 then
    return table.concat(parts, "\n")
  end
  local lower = string.lower(html)
  if string.find(lower, "cf%-challenge") ~= nil or string.find(lower, "just a moment", 1, true) ~= nil then
    error("genius challenge page")
  end
  return nil
end

local function add_candidate(candidates, candidate)
  if #candidates >= 24 then
    return
  end
  if nonempty(candidate.plain_text) or nonempty(candidate.synced_lrc) then
    candidates[#candidates + 1] = candidate
  end
end

local function linked_genius()
  local text = genius_lyrics(inputs.genius_url)
  if text ~= nil then
    return text
  end
  if inputs.genius_song_id == nil then
    return nil
  end
  local song = request_json("https://genius.com/api/songs/" .. urlencode(tostring(inputs.genius_song_id)))
  if song == nil or type(song.response) ~= "table" or type(song.response.song) ~= "table" then
    return nil
  end
  return genius_lyrics(song.response.song.url)
end

local failures = {}
local failed_sources = {}
local genius_blocked = false
local lrclib_busy = false

local function attempt(source, step, ...)
  if failed_sources[source] then
    return nil
  end
  local ok, result = pcall(step, ...)
  if ok then
    return result
  end
  local message = tostring(result)
  failed_sources[source] = true
  failures[#failures + 1] = source .. ": " .. message
  if source == "genius" and (string.find(message, "status 403", 1, true) ~= nil or string.find(message, "challenge page", 1, true) ~= nil) then
    genius_blocked = true
  end
  if source == "lrclib" and string.find(message, "status 5", 1, true) ~= nil then
    lrclib_busy = true
  end
  return nil
end

local linked = attempt("genius", linked_genius)
if linked ~= nil then
  return {
    kind = "found",
    candidates = {
      {
        source = "genius",
        plain_text = linked,
        exact = true,
        query_index = 0
      }
    }
  }
end

local candidates = {}

local function normalized(value)
  if not nonempty(value) then
    return ""
  end
  return (string.gsub(string.lower(value), "[%s%p]", ""))
end

local function settled()
  local title = normalized(inputs.title)
  local artist = normalized(inputs.artist)
  if title == "" then
    return false
  end
  for _, candidate in ipairs(candidates) do
    local candidate_artist = normalized(candidate.artist)
    local same_artist = artist == "" or candidate_artist == ""
      or string.find(candidate_artist, artist, 1, true) ~= nil
      or string.find(artist, candidate_artist, 1, true) ~= nil
    if same_artist and normalized(candidate.title) == title then
      return true
    end
  end
  return false
end

local function add_lrclib_item(item, query_index)
  if type(item) ~= "table" or not (nonempty(item.plainLyrics) or nonempty(item.syncedLyrics)) then
    return false
  end
  add_candidate(candidates, {
    source = "lrclib",
    plain_text = item.plainLyrics,
    synced_lrc = item.syncedLyrics,
    artist = item.artistName,
    title = item.trackName,
    duration_sec = item.duration,
    query_index = query_index
  })
  return true
end

local lrclib_exact_tried = false

local function add_lrclib_exact()
  lrclib_exact_tried = true
  if not nonempty(inputs.title) or not nonempty(inputs.artist) or type(inputs.duration_sec) ~= "number" then
    return
  end
  local item = request_json("https://lrclib.net/api/get?track_name=" .. urlencode(inputs.title)
    .. "&artist_name=" .. urlencode(inputs.artist)
    .. "&duration=" .. tostring(math.floor(inputs.duration_sec)))
  add_lrclib_item(item, 1)
end

local function add_lrclib(query, query_index)
  if not lrclib_exact_tried then
    add_lrclib_exact()
    if settled() or not spare(1) then
      return
    end
  end
  local payload = request_json("https://lrclib.net/api/search?q=" .. urlencode(query))
  if type(payload) ~= "table" then
    error("invalid lrclib payload")
  end
  local added = 0
  for _, item in ipairs(payload) do
    if added >= 3 then
      break
    end
    if add_lrclib_item(item, query_index) then
      added = added + 1
    end
  end
end

local genius_pages = 0

local function add_genius(query, query_index)
  local payload = request_json("https://genius.com/api/search/multi?q=" .. urlencode(query))
  if type(payload) ~= "table" or type(payload.response) ~= "table" then
    error("invalid genius search payload")
  end
  local sections = payload.response.sections
  if type(sections) ~= "table" then
    return
  end
  for _, section in ipairs(sections) do
    if section.type == "song" and type(section.hits) == "table" then
      for _, hit in ipairs(section.hits) do
        local result = hit.result
        if type(result) == "table" and nonempty(result.url) then
          genius_pages = genius_pages + 1
          local text = genius_lyrics(result.url)
          if text ~= nil then
            local artist = nil
            if type(result.primary_artist) == "table" then
              artist = result.primary_artist.name
            end
            add_candidate(candidates, {
              source = "genius",
              plain_text = text,
              artist = artist,
              title = result.title,
              query_index = query_index
            })
          end
          return
        end
      end
    end
  end
end

for index, query in ipairs(inputs.queries or {}) do
  if index > 4 or settled() or not spare(1) then
    break
  end
  if nonempty(query) then
    attempt("lrclib", add_lrclib, query, index)
    if genius_pages < 2 and not settled() and spare(2) then
      attempt("genius", add_genius, query, index)
    end
  end
end

if #candidates == 0 then
  if genius_blocked then
    return { kind = "empty", failed = failures, genius_blocked = true, __verdict = "soft_negative" }
  end
  if lrclib_busy and #failures == 1 then
    return { kind = "empty", failed = failures }
  end
  if #failures > 0 then
    error(table.concat(failures, "; "), 0)
  end
  return { kind = "empty", __verdict = "terminal" }
end

if genius_blocked then
  return { kind = "found", candidates = candidates, failed = failures, genius_blocked = true, __verdict = "soft_negative" }
end

if #failures > 0 then
  return { kind = "found", candidates = candidates, failed = failures }
end

return { kind = "found", candidates = candidates }
