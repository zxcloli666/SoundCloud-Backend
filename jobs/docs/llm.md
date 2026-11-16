# Model questions in jobs

jobs asks a model in two places:

- enrich: who the credited artists of an upload are;
- wanted tracks: which search hit is the same recording as a wanted track.

Both go through a ladder from `utils/llm-ladder`, the crate the api uses for voice chat. Code: `jobs/src/handlers/enrich/ai`, `jobs/src/handlers/wanted/ai_matcher`, `jobs/src/llm`.

## Turn it on

1. Copy `jobs/llm.toml.example` and put your providers in it. The ladder is `refine`.
2. Set `LLM_CONFIG` to the file path and set every variable the file names in `api_key = "env:NAME"`.
3. Restart jobs.

| Setting | Meaning |
| --- | --- |
| `ENRICH_AI_ENABLED` | default `true`. `false`: no model is asked |
| `LLM_CONFIG` | path to the ladder file. Unset: no model is asked. Unreadable, invalid or with a section jobs does not know: jobs does not start |
| `ENRICH_AI_LADDER` | ladder id in the file, default `refine`. Missing from the file: jobs does not start |
| `ENRICH_AI_TIMEOUT_MS` | deadline of one question: 1 to 20 s for enrich, 1 to 10 s for wanted |
| `ENRICH_AI_DAILY_BUDGET` | questions per UTC day for both tasks together, counted in Postgres |

The file has the api format: `[providers]`, `[ladders]`, `[budget]`, see `api/docs/voice-chat.md`. A `worker` step asks the local model on the worker over NATS (`ai.rpc.complete`). Spend caps need `[budget] bucket`; jobs opens it with its own JetStream connection. A question has no user, so only the ladder caps count.

A burst of wanted tracks asks many questions at once. When a provider's rate limit is shared with the api, set `requests_per_minute` on it here: the questions over the limit go to the next step, and voice chat keeps its share.

## Enrich

jobs reads the title, uploader and metadata artist first:

| Found | Confidence |
| --- | --- |
| metadata artist | 0.9 |
| `Artist - Title` | 0.7 |
| uploader that is not a re-upload channel | 0.5 |
| nothing | 0.3 |

At 0.8 and above no model is asked. Below it the model gets the metadata and the parser guess. The answer counts only when every name in it is written word for word in the title, uploader, metadata artist or description; the name is then spelled as there. An answer that drops the artist the parser found, has an album year outside 1900 to 2100 or a confidence outside 0 to 1 is refused, and the next step of the ladder is asked.

## Wanted tracks

Only the search hits jobs could not link or drop itself reach this step. Hits of another version (remix, live, acoustic, instrumental, sped up, slowed) are dropped, the rest get a score from artist, title and duration. A clear winner (0.85 or more and 0.15 ahead) or no hit at 0.5 is decided without a model. Otherwise the model gets the best 10 and may pick one of them or none; any other id is refused and the next step is asked.

Both prompts spell out the JSON answer in words, so a provider that ignores the answer schema still answers in the right shape.

## No answer

The ladder fails or no step gives an answer that counts: enrich keeps its own heuristic; wanted takes the score verdict (best at 0.5 or more and 0.05 ahead, else no match). The question goes back to the daily count.

Daily count used up: enrich keeps its heuristic; wanted puts the track back for 30 minutes and the attempt does not count.

## Cache

Model answers are kept 30 days in `ai_resolver_cache`, keyed by the question input. Answers the worker stored earlier are read the same way.

## Metrics

`jobs_ai_answers_total{method,outcome}`: `method` is `resolve_artist` or `match_track`; `outcome` is `confident` (no question needed), `cached`, `llm`, `fallback` (no answer that counts) or `over_budget`.

`jobs_llm_steps_total`, `jobs_llm_step_duration_seconds`, `jobs_llm_first_output_seconds`, `jobs_llm_tokens_total`, `jobs_llm_spend_micro_usd_total`: one series per ladder step, the same as `api_llm_*` in the api.
