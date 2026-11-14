# Voice speak

`POST /voice/speak` has a line of the assistant voiced on our side, `GET /voice/speak/{ticket}` hands the clip to the app.
Endpoint: `api/src/modules/voice_speak`. Worker lane: `speak` in `backend-contracts/src/worker_contract.rs`.

A worker needs up to 90 s for a line, longer than any request may wait, so the app posts the line, gets a ticket at once and long-polls for the clip. The api keeps nothing in memory and starts no task: the request goes to the lane, the worker's reply lands in a stream, and any api host answers the poll from there.

## Turn it on

1. Add `[voice_speak]` to the file `LLM_CONFIG` points at (see `api/llm.toml.example`).
2. For the phrase cache, create its bucket with an expiry rule and a quota (see Phrases below), add `[voice_speak.phrases]` and set the two variables its keys name. Voicing works without it.
3. Restart the api. jobs creates the streams `SPEAK_RPC` and `SPEAK_REPLY` and the consumer `speak-workers` from the worker contract.

No `[voice_speak]`: both routes answer 503 `voice_speak_unavailable`. No `[voice_speak.phrases]`: there is no phrase cache, every line goes to a worker, `keep` changes nothing and nothing about phrases is logged or counted. An unknown key, a table without one of its keys, an empty value or a key variable that is unset or empty stops the start.

## Config file

`[voice_speak]`

| Key | Meaning |
| --- | --- |
| `requests_per_minute` | `POST` per user, per host. Above zero |
| `max_in_flight` | lines waiting for a worker or being voiced, counted on the lane for all hosts together, default 4. When `SPEAK_RPC` holds that many, a new line gets 503 `voice_speak_busy` |

`[voice_speak.phrases]`, optional: absent means no phrase cache

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
- `keep`: default `false`. `true` for a line said again word for word, such as a greeting: it is looked up in the phrase cache first and stored there once voiced. A line with `keep: false`, or any line when there is no cache, never touches it.
- The body is at most 5 296 bytes, a larger one gets 413.

| Status | Body | Meaning |
| --- | --- | --- |
| 200 | `audio/ogg` | the clip from the phrase cache, only for `keep: true` and only with a cache |
| 202 | `{"id": "0190f0c2-7a4e-7c1d-9b3e-5f2a8c6d4e1b"}` | the line is on the lane. The ticket is a UUID v7 in its hyphenated form; poll it |

The 202 comes only after JetStream has stored the request, and for a kept line the phrase key before it.

## GET /voice/speak/{ticket}

Header `x-session-id`. No Star check, no rate limit. The request holds up to 20 s and looks for the reply twice a second.

| Status | Body | Meaning |
| --- | --- | --- |
| 200 | `audio/ogg` | the clip. The reply stays for 10 minutes, a second poll gets it again |
| 204 | none | not ready yet, poll again |
| 502 | `voice_speak_failed` | the worker refused the line or sent a broken clip, or no reply came within 95 s of the ticket |

A ticket belongs to the user who posted it: a poll by anyone else finds nothing and gets 204. The poll does not take a core permit of the load shedder, like `GET /remote/wait`.

The clip is an Ogg Opus file, mono.

## Errors

JSON, `{"statusCode": 400, "code": "voice_speak_invalid", "message": "text is empty", "error": "Bad Request"}`. Match on `code`.

| Status | `code` | Meaning |
| --- | --- | --- |
| 400 | `voice_speak_invalid` | unknown voice, empty text, text over 300 characters or an account id that cannot be part of a subject, `message` says which. 413, 415 and 422 carry the same code for a body that is too large, has no JSON content type or has the wrong shape |
| 401 | none | no session: `{"statusCode": 401, "message": "Missing or malformed x-session-id header", "error": "Unauthorized"}`, or `"Session not found"` |
| 403 | `star_required` | `POST` only: the user has no Star |
| 404 | `voice_speak_not_found` | `GET` only: the ticket is not a UUID v7 |
| 429 | `voice_speak_limited` | per-user limit, wait for `Retry-After` seconds |
| 502 | `voice_speak_failed` | `GET` only: the line could not be voiced |
| 503 | `voice_speak_unavailable` | no `[voice_speak]`. `Retry-After: 30` |
| 503 | `voice_speak_busy` | the lane is full, or the line could not be stored on it. No `Retry-After`: say the line on the device |
| 503 | `voice_busy` | the rate limiter is unreachable. `Retry-After: 1` |

## Worker

Trusted workers and public nodes behind the bridge serve the lane. A public node that waits for a line gets it before a trusted worker.

Subject `speak.rpc.say`, request `{"voice": "loli", "text": "Hello!"}`, published through JetStream with the rpc headers of the contract: `X-Reply-To` is `speak.reply.<user id>.<ticket>`, `Nats-Msg-Id` is the ticket, `X-Deadline` is the time of the ticket plus 90 s in Unix milliseconds.

The worker publishes its rpc reply to the `X-Reply-To` subject, `{"ok": true, "data": {"seconds": 1.4, "clip": "T2dnUw..."}}` or `{"ok": false, "error": "internal"}`. It needs the right to publish on `speak.reply.>` and nothing else of this stream. `clip` is the standard base64, with padding, of the Ogg file, at most 500 000 characters. A clip that does not decode, is empty or does not start with `OggS` counts as a failure. A line is delivered once and never retried. The full schema is in `backend-contracts/contract/worker-contract.json`.

## Stores

`SPEAK_RPC`: the lane, a work queue of two minutes, at most 16 MiB.

`SPEAK_REPLY`: one JetStream stream in memory, messages live 10 minutes, at most 256 MiB, the oldest go first, read with direct get. Nothing deletes from it by hand.

- `speak.reply.<user id>.<ticket>`: the worker's reply. The poll reads the last message on it.
- `speak.keep.<user id>.<ticket>`: the phrase key of a kept line, published by the api before the request. The first poll that gets a good clip reads it and writes the clip to the phrase cache.

Phrases: the S3 bucket of `[voice_speak.phrases]`, when the table is there.

- Key `<voice>/<sha256>.ogg`, where `<sha256>` is the lowercase hex SHA-256 of the voice, a newline and the trimmed text. Content type `audio/ogg`.
- Only the api reads and writes it. A read waits at most 3 s; a slow or failing read is a miss and the line goes to a worker. The write happens inside the poll and waits at most 2 s.
- The api never deletes phrases. Set the lifetime as an expiry rule and the total size as a quota on the bucket. A refused write, a full quota included, is counted and logged; the poll answers the clip all the same.
- After a voice's model is replaced, delete the prefix `<voice>/`, or the old clips keep being served.

## Metrics

| Metric | Labels |
| --- | --- |
| `api_voice_speak_lines_total` | `outcome`: `cached`, `asked`, `busy` |
| `api_voice_speak_polls_total` | `outcome`: `clip`, `failed`, `expired`, `pending` |
| `api_voice_speak_replies_total`, `api_voice_speak_reply_duration_seconds` | `outcome`: `ok`, `expired`, `invalid`, `internal`, `rejected`, `malformed`. Counted by every poll that finds a reply; the duration runs from the ticket to that poll |
| `api_voice_speak_clip_seconds` | none: length of a voiced clip |
| `api_voice_speak_store_errors_total` | `store`: `lane`, `replies`, `phrases`; `action`: `depth`, `ask`, `keep`, `read`, `write`; `error`: `unavailable`, `timeout`, `malformed` |

Logs carry stores, actions, reasons and codes, never the text, a phrase key, a subject or a ticket.
