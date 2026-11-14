# Voice speak

`POST /voice/speak` has a line of the assistant voiced on our side, `GET /voice/speak/{ticket}` hands the clip to the app.
Endpoint: `api/src/modules/voice_speak`. Worker lane: `speak` in `backend-contracts/src/worker_contract.rs`.

A worker needs up to 90 s for a line, longer than any request may wait, so the app posts the line, gets a ticket at once and long-polls for the clip.

## Turn it on

1. Add `[voice_speak]` and `[voice_speak.phrases]` to the file `LLM_CONFIG` points at (see `api/llm.toml.example`) and set the two variables the keys name.
2. Create the phrase bucket with an expiry rule and a quota (see Phrases below).
3. Restart the api. jobs creates the stream `SPEAK_RPC`, the consumer `speak-workers` and the store `SPEAK_CLIPS` from the worker contract.

No `[voice_speak]`: both routes answer 503 `voice_speak_unavailable`. An unknown key, an empty value or a key variable that is unset or empty stops the start.

## Config file

`[voice_speak]`

| Key | Meaning |
| --- | --- |
| `requests_per_minute` | `POST` per user, per host. Above zero |
| `max_in_flight` | lines one api process has with the workers at once, default 4. A line over it gets 503 `voice_speak_busy` |

`[voice_speak.phrases]`, required

| Key | Meaning |
| --- | --- |
| `endpoint` | URL of the S3 endpoint, `http` or `https`. Path-style addressing |
| `bucket` | bucket of the phrase cache |
| `access_key`, `secret_key` | `env:NAME`, read like a provider `api_key` |

## POST /voice/speak

Header `x-session-id`. The user needs Star. The per-user limit and the Star check run before the body is read.

```json
{ "voice": "loli", "text": "Hello!", "keep": true }
```

- `voice`: `loli` or `moriarty`.
- `text`: trimmed, 1 to 300 characters.
- `keep`: default `false`. `true` for a line said again word for word, such as a greeting: it is looked up in the phrase cache first and stored there once voiced. A line with `keep: false` never touches the cache.
- The body is at most 5 296 bytes, a larger one gets 413.

| Status | Body | Meaning |
| --- | --- | --- |
| 200 | `audio/ogg` | the clip from the phrase cache, only for `keep: true` |
| 202 | `{"id": "0190f0c2-7a4e-7c1d-9b3e-5f2a8c6d4e1b"}` | the line went to a worker. The ticket is a UUID in its hyphenated form; poll it |

## GET /voice/speak/{ticket}

Header `x-session-id`. No Star check, no rate limit. The request holds up to 20 s and looks for the clip twice a second.

| Status | Body | Meaning |
| --- | --- | --- |
| 200 | `audio/ogg` | the clip. It stays for 10 minutes, a second poll gets it again |
| 204 | none | not ready yet, poll again. The api waits 90 s for a worker, after that the poll gets the clip or 502 |
| 502 | `voice_speak_failed` | the worker failed, timed out or sent a broken clip, or the api could not store it |

A ticket belongs to the user who posted it: a poll by anyone else finds nothing and gets 204. The poll does not take a core permit of the load shedder, like `GET /remote/wait`.

The clip is an Ogg Opus file, mono.

## Errors

JSON, `{"statusCode": 400, "code": "voice_speak_invalid", "message": "text is empty", "error": "Bad Request"}`. Match on `code`.

| Status | `code` | Meaning |
| --- | --- | --- |
| 400 | `voice_speak_invalid` | unknown voice, empty text or text over 300 characters, `message` says which. 413, 415 and 422 carry the same code for a body that is too large, has no JSON content type or has the wrong shape |
| 401 | none | no session: `{"statusCode": 401, "message": "Missing or malformed x-session-id header", "error": "Unauthorized"}`, or `"Session not found"` |
| 403 | `star_required` | `POST` only: the user has no Star |
| 404 | `voice_speak_not_found` | `GET` only: the ticket is not a UUID |
| 429 | `voice_speak_limited` | per-user limit, wait for `Retry-After` seconds |
| 502 | `voice_speak_failed` | `GET` only: the line could not be voiced |
| 503 | `voice_speak_unavailable` | no `[voice_speak]`. `Retry-After: 30` |
| 503 | `voice_speak_busy` | `max_in_flight` lines of this host are with the workers. No `Retry-After`: say the line on the device |
| 503 | `voice_busy` | the rate limiter is unreachable. `Retry-After: 1` |

## Worker

Subject `speak.rpc.say`, request `{"voice": "loli", "text": "Hello!"}`, reply data `{"seconds": 1.4, "clip": "T2dnUw..."}`. `clip` is the standard base64, with padding, of the Ogg file, at most 500 000 characters. A clip that does not decode, is empty or does not start with `OggS` counts as a failure. The worker has 90 s, a line is delivered once and never retried. The full schema is in `backend-contracts/contract/worker-contract.json`.

## Stores

`SPEAK_CLIPS`: a NATS object store in memory, objects live 10 minutes, at most 256 MiB. Only the api uses it, as the hand-over between the process that asked the worker and the process that answers the poll, which may run on another host. The asking process writes the clip under `<user id>.<ticket>`, or an empty object as the failure mark. Nothing deletes clips by hand.

Phrases: the S3 bucket of `[voice_speak.phrases]`.

- Key `<voice>/<sha256>.ogg`, where `<sha256>` is the lowercase hex SHA-256 of the voice, a newline and the trimmed text. Content type `audio/ogg`.
- Only the api reads and writes it. A read waits at most 3 s; a slow or failing read is a miss and the line goes to a worker. A write waits at most 10 s.
- The api never deletes phrases. Set the lifetime as an expiry rule and the total size as a quota on the bucket. A refused write, a full quota included, is counted and logged; the line is answered all the same.
- After a voice's model is replaced, delete the prefix `<voice>/`, or the old clips keep being served.

## Metrics

| Metric | Labels |
| --- | --- |
| `api_voice_speak_lines_total` | `outcome`: `cached`, `asked`, `busy` |
| `api_voice_speak_replies_total`, `api_voice_speak_reply_duration_seconds` | `outcome`: `ok`, `timeout`, `unreachable`, `rejected`, `invalid`, `malformed`, `unencodable` |
| `api_voice_speak_clip_seconds` | none: length of a voiced clip |
| `api_voice_speak_polls_total` | `outcome`: `clip`, `failed`, `pending` |
| `api_voice_speak_store_errors_total` | `store`: `clips`, `phrases`; `action`: `read`, `write`, `mark`; `error`: `unavailable`, `timeout` |

Logs carry lengths, sizes and codes, never the text, a phrase key or a clip name.
