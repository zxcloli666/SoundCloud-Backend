local browser_ua = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36"

local function request(url)
  local headers = {
    ["Accept-Encoding"] = "identity",
    ["Accept"] = "application/json,text/html;q=0.9,*/*;q=0.8",
    ["Accept-Language"] = "en-US,en;q=0.9",
    ["User-Agent"] = browser_ua
  }
  if string.find(url, "musixmatch.com", 1, true) ~= nil then
    headers["Cookie"] = "x-mxm-token-guid=" .. client_id()
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
  local lower = string.lower(html)
  if string.find(lower, "cf%-challenge") ~= nil or string.find(lower, "captcha") ~= nil or string.find(lower, "just a moment", 1, true) ~= nil then
    error("genius challenge page")
  end
  local parts = {}
  for container in string.gmatch(html, '<div[^>]-data%-lyrics%-container="true"[^>]*>(.-)</div>') do
    local text = decode_html(container)
    if nonempty(text) and #text > 20 then
      parts[#parts + 1] = text
    end
  end
  if #parts == 0 then
    return nil
  end
  return table.concat(parts, "\n")
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

local linked = linked_genius()
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
local mxm_token = nil
local mxm_token_loaded = false
local mxm_base = inputs.musixmatch_base

local function get_mxm_token()
  if mxm_token_loaded then
    return mxm_token
  end
  mxm_token_loaded = true
  if not nonempty(mxm_base) then
    error("musixmatch base is missing")
  end
  local payload = request_json(mxm_base .. "/token.get?app_id=web-desktop-app-v1.0&user_language=en&t=" .. urlencode(client_id()))
  if payload == nil or type(payload.message) ~= "table" or type(payload.message.body) ~= "table" then
    error("invalid musixmatch token payload")
  end
  local token = payload.message.body.user_token
  if not nonempty(token) or token == "UpgradeOnlyUpgradeOnlyUpgradeOnlyUpgradeOnly" then
    error("musixmatch token is unavailable")
  end
  mxm_token = token
  return mxm_token
end

local function add_lrclib(query, query_index)
  local payload = request_json("https://lrclib.net/api/search?q=" .. urlencode(query))
  if type(payload) ~= "table" then
    error("invalid lrclib payload")
  end
  local added = 0
  for _, item in ipairs(payload) do
    if added >= 3 then
      break
    end
    if type(item) == "table" and (nonempty(item.plainLyrics) or nonempty(item.syncedLyrics)) then
      add_candidate(candidates, {
        source = "lrclib",
        plain_text = item.plainLyrics,
        synced_lrc = item.syncedLyrics,
        artist = item.artistName,
        title = item.trackName,
        duration_sec = item.duration,
        query_index = query_index
      })
      added = added + 1
    end
  end
end

local function add_genius(query, query_index)
  local payload = request_json("https://genius.com/api/search/multi?q=" .. urlencode(query))
  if type(payload) ~= "table" or type(payload.response) ~= "table" then
    error("invalid genius search payload")
  end
  local added = 0
  local sections = payload.response.sections or {}
  for _, section in ipairs(sections) do
    if section.type == "song" and type(section.hits) == "table" then
      for _, hit in ipairs(section.hits) do
        if added >= 2 then
          return
        end
        local result = hit.result
        if type(result) == "table" and nonempty(result.url) then
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
            added = added + 1
          end
        end
      end
    end
  end
end

local function deep_find(value, key)
  if type(value) ~= "table" then
    return nil
  end
  if value[key] ~= nil then
    return value[key]
  end
  for _, child in pairs(value) do
    local found = deep_find(child, key)
    if found ~= nil then
      return found
    end
  end
  return nil
end

local function normalize_subtitle(raw)
  if not nonempty(raw) or string.find(raw, "%[%d+:%d+") == nil then
    return nil
  end
  raw = string.gsub(raw, "^%s+", "")
  raw = string.gsub(raw, "%s+$", "")
  return raw
end

local function richsync_to_lrc(raw)
  if not nonempty(raw) then
    return nil
  end
  local entries = json_decode(raw)
  if type(entries) ~= "table" then
    return nil
  end
  local output = {}
  for _, entry in ipairs(entries) do
    if type(entry) == "table" and type(entry.ts) == "number" then
      local words = {}
      if type(entry.l) == "table" then
        for _, word in ipairs(entry.l) do
          if type(word) == "table" and nonempty(word.c) then
            words[#words + 1] = word.c
          end
        end
      end
      local text = table.concat(words, " ")
      if not nonempty(text) and nonempty(entry.x) then
        text = entry.x
      end
      if nonempty(text) then
        local minute = math.floor(entry.ts / 60)
        local second = entry.ts - minute * 60
        output[#output + 1] = string.format("[%02d:%05.2f] %s", minute, second, text)
      end
    end
  end
  if #output == 0 then
    return nil
  end
  return table.concat(output, "\n")
end

local function clean_plain(plain)
  if not nonempty(plain) then
    return nil
  end
  local lower = string.lower(plain)
  local disclaimer = string.find(lower, "this lyrics is not for commercial use", 1, true)
  local stars = string.find(plain, "*****", 1, true)
  local cutoff = nil
  if disclaimer ~= nil then cutoff = disclaimer end
  if stars ~= nil and (cutoff == nil or stars < cutoff) then cutoff = stars end
  if cutoff ~= nil then plain = string.sub(plain, 1, cutoff - 1) end
  plain = string.gsub(plain, "^%s+", "")
  plain = string.gsub(plain, "%s+$", "")
  if #plain <= 20 then
    return nil
  end
  return plain
end

local function add_musixmatch(query_index)
  local token = get_mxm_token()
  local duration = tonumber(inputs.duration_sec) or 0
  local url = mxm_base .. "/macro.subtitles.get?app_id=web-desktop-app-v1.0&usertoken=" .. urlencode(token) .. "&namespace=lyrics_richsynched&subtitle_format=lrc&q_track=" .. urlencode(inputs.title or "") .. "&q_artist=" .. urlencode(inputs.artist or "") .. "&q_album=&q_duration=" .. tostring(duration) .. "&optional_calls=track.richsync&format=json"
  local payload = request_json(url)
  if payload == nil or type(payload.message) ~= "table" or type(payload.message.header) ~= "table" then
    error("invalid musixmatch macro payload")
  end
  if tonumber(payload.message.header.status_code) ~= 200 then
    error("musixmatch macro status " .. tostring(payload.message.header.status_code))
  end
  local root = type(payload.message.body) == "table" and payload.message.body.macro_calls or payload
  local track = deep_find(root, "track")
  if type(track) == "table" and tonumber(track.instrumental) ~= 1 then
    local synced = richsync_to_lrc(deep_find(root, "richsync_body"))
    if synced == nil then
      synced = normalize_subtitle(deep_find(root, "subtitle_body"))
    end
    local plain = clean_plain(deep_find(root, "lyrics_body"))
    add_candidate(candidates, {
      source = "musixmatch",
      plain_text = plain,
      synced_lrc = synced,
      artist = track.artist_name or inputs.artist,
      title = track.track_name or inputs.title,
      duration_sec = track.track_length,
      query_index = query_index
    })
  end
end

add_musixmatch(0)

for index, query in ipairs(inputs.queries or {}) do
  if index > 4 then
    break
  end
  if nonempty(query) then
    add_lrclib(query, index)
    add_genius(query, index)
  end
end

if #candidates == 0 then
  return { kind = "empty", candidates = {}, __verdict = "terminal" }
end

return { kind = "found", candidates = candidates }
