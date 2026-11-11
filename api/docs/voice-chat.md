# Voice chat

`POST /voice/chat` answers a spoken phrase with a model: actions for the app and text to speak, sent as an event stream.
Endpoint: `api/src/modules/voice_chat`. Model ladder: `utils/llm-ladder`.

## Turn it on

1. Copy `api/llm.toml.example` and fill in your providers, ladder and personas.
2. Set `LLM_CONFIG` to the file path and set every variable the file names in `api_key = "env:NAME"`.
3. Restart the api.

`LLM_CONFIG` unset: the endpoint answers 503 `chat_unavailable`. An unknown key or a bad value stops the start.
Spend caps need NATS JetStream: the api creates the KV bucket named in `[budget]` on first use.

## Config file

`[providers.<id>]`

| Key | Meaning |
| --- | --- |
| `kind` | `anthropic_compatible`, `openai_compatible` or `worker` |
| `endpoint`, `model`, `api_key` | http kinds. The key is always `env:NAME`; an unset or empty variable turns the provider off |
| `stream` | http kinds, default `true`. `false` = one call, the answer arrives as one piece |
| `max_output_tokens`, `total_timeout_ms` | limits of one call |
| `input_usd_per_mtok`, `output_usd_per_mtok` | optional price. A provider with a price counts against the spend caps |
| `body` | optional JSON object merged into the request body, for provider-specific knobs |
| `subject` | `worker` only: NATS subject of the local model |

`[ladders.<id>]`

| Key | Meaning |
| --- | --- |
| `steps` | provider ids, most wanted first |
| `hedge_after_ms` | the next step starts when the running ones gave no output for this long, or at once when a step fails |
| `deadline_ms` | limit for the whole answer |
| `breaker` | `failures` within `window_s` seconds skip the provider for `open_s` seconds |
| `[ladders.<id>.budget]` | optional caps in USD: `month_usd`, `day_usd`, `user_month_usd`, `user_day_usd`. A reached cap skips priced steps, free steps keep answering |

`[budget] bucket`: NATS KV bucket with the spend counters, shared by every host. Counters are whole micro-USD, UTC periods, kept 40 days.

`[voice_chat]`

| Key | Meaning |
| --- | --- |
| `ladder` | ladder id |
| `requests_per_minute` | per user, per host |
| `max_text_chars`, `max_history`, `max_notes`, `max_tools` | request caps |
| `max_tool_rounds` | requests one turn may take. A request that may be the last one (it carries at least `max_tool_rounds - 1` results of `returns: true` tools) is offered no such tools, so the model has to answer |
| `instructions` | system text added for every persona |
| `[voice_chat.personas.<id>] prompt` | who the character is. `default` is required and serves unknown ids |

## Request

Header `x-session-id`. The user needs Star. JSON body, only `text` is required:

```json
{
  "text": "make it louder",
  "persona": "loli",
  "language": "en",
  "context": { "volume": 40 },
  "history": [ { "role": "user", "text": "hi" }, { "role": "assistant", "text": "hello" } ],
  "notes": [ "likes jazz" ],
  "tools": [
    {
      "name": "volume_set",
      "description": "Set the playback volume.",
      "args": { "type": "object", "properties": { "level": { "type": "integer", "minimum": 0, "maximum": 100 } }, "required": [ "level" ] },
      "returns": false
    }
  ],
  "results": [ { "name": "search_tracks", "args": { "query": "night" }, "result": { "tracks": [ ] } } ]
}
```

- Tool and argument names: `^[a-z][a-z0-9_]{0,31}$`, unique. Arguments are flat: `string`, `integer`, `number`, `boolean` or `enum` of strings; `description`, `minimum` and `maximum` are read, other keywords are ignored. At most 16 arguments per tool.
- `context` and `results`: at most 8 KiB each as JSON, at most 16 results. A history item: at most 2000 characters. A note: at most 300. A description: at most 600.
- `results` are what the tools of this turn answered: send the same `text` and `history` again with them.

## Untrusted text

Track and playlist names, search results, notes and earlier messages can carry text written to look like an order. The system text tells the model that only its rules and the latest `text` are instructions; `context`, `notes` and `results` reach it inside `<app_state>`, `<notes>` and `<tool_results>` blocks with `<` and `>` escaped, so a value cannot close its block.

Check it against the real providers (needs `LLM_CONFIG`, its keys and `NATS_URL`):

```sh
cargo test --bin api -- --ignored --nocapture modules::voice_chat::live_tests
```

## Response

`200 text/event-stream`, never compressed, a comment line every 15 s while nothing else is sent:

| Event | Data | When |
| --- | --- | --- |
| `action` | `{"name": "volume_set", "args": {"level": 60}}` | once per action, as soon as it is complete. `args` is always an object with the declared types; an optional argument the model did not give is absent. Actions that fit no offered tool are left out |
| `say` | `{"text": "piece"}` | pieces of the spoken text, in order |
| `done` | `{}` | the turn is complete |
| `error` | `{"code": "provider_failed"}` or `{"code": "deadline"}` | the answer broke after the stream started |

Closing the connection cancels the provider calls.

Errors before the stream are JSON, `{"statusCode": 403, "code": "star_required", "message": "...", "error": "Forbidden"}`. Match on `code`; an answer without it did not come from this endpoint.

| Status | `code` | Meaning |
| --- | --- | --- |
| 400 | `voice_chat_invalid` | the body is over a cap or is not JSON, `message` says which field. 413, 415 and 422 carry the same code for a body that is too large, has no JSON content type or has the wrong shape |
| 401 | none | no session: `{"statusCode": 401, "message": "Missing or malformed x-session-id header", "error": "Unauthorized"}`, or `"Session not found"` |
| 403 | `star_required` | the user has no Star |
| 429 | `voice_chat_limited` | per-user limit, wait for `Retry-After` seconds |
| 503 | `chat_unavailable` | no config, no provider with a key, every breaker open, or the caps are spent and no free step exists. `Retry-After: 30` |
| 503 | `voice_busy` | the rate limiter is unreachable. `Retry-After: 1` |

## Metrics

`api_llm_steps_total`, `api_llm_step_duration_seconds`, `api_llm_first_output_seconds`, `api_llm_tokens_total`, `api_llm_spend_micro_usd_total` (labels `ladder`, `provider`, `result`), `api_voice_chat_turns_total`, `api_voice_chat_turn_duration_seconds`, `api_voice_chat_first_event_seconds`, `api_voice_chat_actions_total`.

Logs carry provider ids, statuses and sizes, never the phrase or the answer.
