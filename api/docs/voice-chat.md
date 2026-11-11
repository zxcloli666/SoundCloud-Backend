# Voice chat

`POST /voice/chat` answers a spoken phrase with a model: actions for the app and text to speak, sent as an event stream.
Endpoint: `api/src/modules/voice_chat`. Model ladder: `utils/llm-ladder`.

## Turn it on

1. Copy `api/llm.toml.example` and fill in your providers, ladder and personas.
2. Set `LLM_CONFIG` to the file path and set every variable the file names in `api_key = "env:NAME"`.
3. Restart the api.

`LLM_CONFIG` unset: the endpoint answers 503 `chat_unavailable`. An unknown key or a bad value stops the start.
Spend caps need NATS JetStream: the api opens the KV bucket named in `[budget]` and creates it only when it is missing; it never changes an existing bucket. NATS unreachable: priced steps are skipped for 10 s, one warning per pause.

## Config file

`[providers.<id>]`

| Key | Meaning |
| --- | --- |
| `kind` | `anthropic_compatible`, `openai_compatible` or `worker` |
| `endpoint`, `model`, `api_key` | http kinds. The key is always `env:NAME`; an unset or empty variable turns the provider off |
| `stream` | http kinds, default `true`. `false` = one call, the answer arrives as one piece |
| `max_output_tokens`, `total_timeout_ms` | limits of one call. A `worker` takes at most 1024 tokens |
| `input_usd_per_mtok`, `output_usd_per_mtok` | optional price. A provider with a price counts against the spend caps |
| `cache_read_usd_per_mtok`, `cache_write_usd_per_mtok` | optional price of input the provider read from its prefix cache or wrote to it. Missing = `input_usd_per_mtok` |
| `prefix_cache` | `anthropic_compatible` only: the object sent as `cache_control` on the system blocks that end a shared prefix, for example `{ type = "ephemeral" }`. Missing = the system text goes as one string and nothing is cached |
| `body` | optional JSON object merged into the request body, for provider-specific knobs |
| `headers` | http kinds: static request headers, for example the API version a vendor needs: `{ "anthropic-version" = "..." }`. `authorization`, `x-api-key`, `content-type`, `content-length` and `host` are refused |
| `hedge` | default `true`. `false`: the step starts only when every earlier step has failed, never because they are silent. Set it on a model that must not get parallel calls, such as a local CPU model |
| `subject` | `worker` only: NATS subject of the local model |

A `worker` step gets the request cut to the limits of the worker method: the newest 64 messages, the first of them a user message, and the newest 12 000 bytes of a longer message. It fails without a publish only when the system text is over 48 000 bytes or the schema over 64 000 bytes. The call carries `X-Deadline`: the time the ladder has left, at most `total_timeout_ms`. A started worker call runs until it answers or reaches that deadline, even when another step has already won.

`[ladders.<id>]`

| Key | Meaning |
| --- | --- |
| `steps` | provider ids, most wanted first |
| `hedge_after_ms` | the next step starts when the running ones gave no output for this long, or at once when a step fails |
| `deadline_ms` | limit for the whole answer. Every step's `total_timeout_ms` must be below it |
| `breaker` | `failures` within `window_s` seconds skip the provider for `open_s` seconds, then one call probes |
| `[ladders.<id>.budget]` | optional caps in USD: `month_usd`, `day_usd`, `user_month_usd`, `user_day_usd`. A reached cap skips priced steps, free steps keep answering |

`[budget] bucket`: NATS KV bucket with the spend counters, shared by every host. Counters are whole micro-USD, UTC periods, kept 40 days.

Times are at most 600 000 ms, `window_s` and `open_s` at most 86 400.

The breaker counts only faults of the provider: a transport error, a stream that closes before its answer ended, a timeout, HTTP 5xx, 429, 401, 403, an overloaded error, and a streaming step that gave no output for longer than `hedge_after_ms` when the deadline came. A step that cannot stream (a `worker`, `stream = false`) is judged by its own `total_timeout_ms` only. While a breaker is open only its probe call closes or reopens it; a call that started earlier changes nothing. A refusal, an answer cut at `max_output_tokens`, HTTP 400, 413 or 422, a worker `invalid_request` and an answer that is not the JSON document fail the step for this request only; the ladder goes on to the next step.

A priced call that ends without its final usage (a loser, a client that left, a deadline, a broken stream, a provider that never sends it) is charged an estimate, 3 bytes per token. The input and cache counts it reported are kept, or come from the request size when it reported none; the output is the larger of the reported count and the bytes it sent. Only a call the provider did not accept (a status other than 2xx, a refused worker call, no connection) costs nothing.

`[voice_chat]`

| Key | Meaning |
| --- | --- |
| `ladder` | ladder id |
| `requests_per_minute` | per user, per host |
| `max_text_chars`, `max_history`, `max_notes`, `max_tools` | request caps |
| `max_system_bytes` | cap on the system text the tools, notes and app state make, in bytes. A request over it gets 400. At most 48 000 when the ladder has a `worker` step |
| `max_tool_rounds` | requests one turn may take. A request that may be the last one (it carries at least `max_tool_rounds - 1` results of `returns: true` tools) is offered no such tools, so the model has to answer |
| `instructions` | system text added for every persona |
| `[voice_chat.personas.<id>] prompt` | who the character is. `default` is required and serves unknown ids |

## Prompt order and the prefix cache

The system text has three sections, the most shared first:

1. the same for every user of one app version: setting, `instructions`, reply format, tool list in the order the app sent it, the rule about data;
2. per character: persona prompt, language;
3. per turn: notes, app state.

A provider that caches by prefix reuses sections 1 and 2. With `prefix_cache` set, sections 1 and 2 each end with a cache mark: a turn of a known character reads both from the cache, a new character reads section 1 and writes its own section 2.
`openai_compatible` and `worker` get the same text joined in the same order.
A change to `instructions`, a persona prompt or the app's tool list starts a new cache entry, so keep per-user and per-turn text out of them. The last tool round is offered fewer tools and has an entry of its own.

## Request

Header `x-session-id`. The user needs Star. The Star check and the per-user limit run before the body is read. JSON body, only `text` is required:

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

- `persona`: a `[voice_chat.personas.<id>]` id, an unknown one gets `default`. `language`: the language the app listens and speaks in, the reply is written in it; without it the reply follows the language of `text`.
- Tool and argument names: `^[a-z][a-z0-9_]{0,31}$`, unique. Arguments are flat: `string`, `integer`, `number`, `boolean` or `enum` of strings; `description`, `minimum` and `maximum` are read, other keywords are ignored. At most 16 arguments per tool.
- `context` and `results`: at most 8 KiB each as JSON, at most 16 results. A description: at most 600 characters.
- `history` and `notes`: only the newest `max_history` items and `max_notes` notes are kept; a longer history item is cut to 2000 characters, a note to 300. A wrong shape is still refused.
- The body may be as large as the caps allow: 4 bytes per counted character or byte plus 4 KiB, at most 256 KiB. A larger one gets 413.
- `results` are what the tools of this turn answered: send the same `text` and `history` again with them.

## Untrusted text

Track and playlist names, search results, notes and earlier messages can carry text written to look like an order. The system text tells the model that only its rules and the latest `text` are instructions; `context`, `notes` and `results` reach it inside `<app_state>`, `<notes>` and `<tool_results>` blocks with `<` and `>` escaped, so a value cannot close its block. Tool descriptions and enum values are escaped the same way.

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
| `error` | `{"code": "provider_failed"}` or `{"code": "deadline"}` | the answer broke after the stream started, or no step gave an action that fits a tool or any speech |

A step's answer reaches the client from its first action that fits a tool or its first spoken word. An answer with neither falls through to the next step; the step gets up to 1 s to send its usage first.

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

`api_llm_steps_total`, `api_llm_step_duration_seconds`, `api_llm_first_output_seconds`, `api_llm_tokens_total`, `api_llm_spend_micro_usd_total` (labels `ladder`, `provider`; steps and durations add `result`), `api_voice_chat_turns_total`, `api_voice_chat_turn_duration_seconds`, `api_voice_chat_first_event_seconds`, `api_voice_chat_actions_total`.

`result`: `won`, `lost`, `cancelled`, `failed_status`, `failed_transport`, `failed_timeout` (these three feed the breaker), `failed_request`, `failed_refused`, `failed_length`, `failed_invalid`, `skipped_breaker`, `skipped_budget`.

A steady `failed_request` rate on one provider means it refuses every request with HTTP 400, 413 or 422, for example an account out of credit: the breaker does not count it, so every turn loses one call to that step until the account or the config is fixed.

`api_llm_tokens_total` and `api_llm_spend_micro_usd_total` have the label `basis`: `reported` by the provider or `estimated` by the api.

`api_llm_tokens_total` has the label `kind`: `input` (full price), `cache_read`, `cache_write`, `output`. The prompt size is `input + cache_read + cache_write`. A working prefix cache shows `cache_read` far above `cache_write`; `cache_write` near the prompt size on every call means the shared sections change between calls.

Logs carry provider ids, statuses and sizes, never the phrase or the answer.
