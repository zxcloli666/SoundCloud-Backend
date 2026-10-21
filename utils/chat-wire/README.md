# chat-wire

Wire format and crypto of the SoundCloud Desktop chat. The server (`api`, role `rt`) and every client use this crate or follow this file byte for byte. Desktop pins it by git tag (`chat-wire-v0.1.1`). Android and iOS implement the same steps and must pass every file in `vectors/`.

All integers inside byte strings are big-endian. `||` is concatenation. `hex()` is lowercase hex. `sha256()` is SHA-256.

## Primitives

| Use | Primitive |
|---|---|
| Sealed requests, CK delivery | HPKE base mode (RFC 9180): DHKEM(X25519, HKDF-SHA256), HKDF-SHA256, ChaCha20Poly1305 |
| Answers, CK wrap, message bodies | XChaCha20-Poly1305, output `nonce[24] || ciphertext` |
| Request and bind signatures | Ed25519 (strict verification) |
| Protobuf | `proto/inner.proto`, `proto/envelope.proto`, package `scd.chat.v1` |

## Origin chat key (OCK)

- X25519. The server holds the private keys, clients carry the public keys as pins (`ock::OCK_PINS`).
- A build pins the current key and the next one. `OCK_PINS[0]` is current when the build ships; `GET /chat/config` answers `ock_current`, and the client switches once that version is pinned.

| Version | Public key |
|---|---|
| 1 | `46b0bccd368d8553a5e09bc8aff285142c58be72454504d01dd7cd964991ee63` |
| 2 | `a4ba96381c8a03bf977799af43ae573eba75d787a47acb7cc2b6622595d43054` |

## Request: `POST /chat/x`

Body (`application/octet-stream`, 64 KB at most):

```
v:u8 = 1 | ock_v:u8 | enc[32] | ct
```

- `enc, ctx = HPKE.SetupBaseS(pk = pin(ock_v), info = "scd-chat/x/v1")`.
- `ct = ctx.Seal(aad = v || ock_v, pt = Inner)`, one message per context.
- `k_resp = ctx.Export("scd-chat/resp/v1", 32)`. Both sides derive it; nobody sends it.
- A fresh ephemeral key per request. Vectors fix it with `ikm_e` (RFC 9180 DeriveKeyPair) and also list `sk_e`.

`Inner` fields:

| Field | Meaning |
|---|---|
| `ts_ms` | client clock, Unix ms; the server accepts +-120 s and otherwise answers `chat_clock {server_now}` |
| `session_id` | music session id; set on every `/chat/...` call, on a sealed relogin (the target session), on a sealed push `/auth/link/create` and on a sealed pull `/auth/link/claim` (the source session); absent on a first login and on the new device's link step |
| `device_id` | chat device id (uuid); absent before the device exists |
| `method`, `path`, `query` | inner HTTP request; `query` without `?` |
| `body` | inner body, JSON bytes or empty |
| `tier` | `TIER_DIRECT`, `TIER_RELAY`, `TIER_PRO` |
| `max_bytes` | answer budget the client can take on this tier |
| `bind` | device keys, only on sealed login and link calls |
| `sig` | Ed25519 signature, below |

Signature string, signed by the device auth key (by `bind.auth_pub` when there is no device yet):

```
scd-chat/req/v2\n{method}\n{path}\n{query}\n{ts_ms}\n{hex(sha256(body))}\n{session_id or -}\n{device_id or -}\n{hex(sha256(enc))}
```

None of the fields may contain `\n`. Binding `enc` stops a signature from moving into another envelope. The server keeps `sha256(enc)[..16]` for 300 s and answers a repeat with `chat_replay`.

Sealed auth paths: `/auth/login`, `/auth/login/status`, `/auth/link/create`, `/auth/link/claim`, `/auth/link/status`. Everything else is `/chat/...`.

Both link modes are sealed paths on both devices:

| Mode | New device (no session) | Source device (bound session) |
|---|---|---|
| pull | sealed `/auth/link/create {mode: pull}` with its `bind` | sealed `/auth/link/claim` with `session_id` |
| push | sealed `/auth/link/claim` with its `bind` | sealed `/auth/link/create {mode: push}` with `session_id` |

The new session gets a chat bind only when the source step is sealed and signed by the key bound to the source session. A plain source step still links, but the new session is unbound and shows the relogin card. A push link is unbound when the source session was rebound between create and claim.

Session gates:

- An unbound session (no bind) reaches only `GET /chat/config`; everything else answers `chat_relogin_required`.
- A bound session without a device row reaches only `GET /chat/config`, `GET /chat/devices` and `POST /chat/devices` (register); everything else answers `chat_device_revoked`.
- `GET /chat/config` answers `user_id`, the caller's own SoundCloud user id. Clients use it for own messages, own read marks and notifications.

## Bind

```
Bind {device_id, auth_pub[32], box_pub[32], ts_ms, sig}
sig = Ed25519(auth key, "scd-chat/bind/v1\n{device_id}\n{hex(auth_pub)}\n{hex(box_pub)}\n{ts_ms}")
```

The server stores the protobuf bytes in `login_requests.chat_bind`, `link_requests.chat_bind` and `sessions.chat_bind`. A session id without a matching bind never reaches chat.

## Answer

- Outer status is 200 for every inner result. 400 means a bad envelope, 503 means busy (with `retry-after`).
- Body: `nonce[24] || XChaCha20-Poly1305(k_resp, InnerResponse, aad = enc)`.
- `InnerResponse {status, headers[], body}` carries the inner HTTP answer.
- A relay can neither read nor forge an answer: only the holder of the OCK private key can derive `k_resp`.

## Inner bodies

- JSON. Byte fields are base64, standard alphabet, no padding.
- Errors are `{"error": "chat_<code>", ...}` with the inner HTTP status: `not_member`, `epoch_stale`, `muted {until}`, `slow_mode {retry_ms}`, `unavailable`, `request_limit`, `send_limited`, `read_limited`, `busy {retry_ms}`, `clock {server_now}`, `replay`, `device_revoked`, `relogin_required`, `session_bound_other`, `too_young`, `disabled`, `send_paused`, `invalid`, `not_found`, `unimplemented`, `internal`.

## Conversation keys (CK)

32 random bytes per `(conversation, epoch)`.

Server storage (clients never see it):

```
wrapped = XChaCha20-Poly1305(KEK_v, CK, aad = "scd-chat/ck/v1" || conv[16] || epoch:u32 || kek_v:u16)
```

Delivery to a device, inside a sealed answer:

```
enc, ctx = HPKE.SetupBaseS(pk = box_pub, info = "scd-chat/ck-seal/v1" || conv[16] || epoch:u32 || device_id[16])
sealed = enc[32] || ctx.Seal(aad = "", pt = CK)
```

`conv` and `device_id` are the 16 raw uuid bytes.

## Messages

```
body = nonce[24] || XChaCha20-Poly1305(CK_epoch, padded Envelope,
       aad = "scd-chat/msg/v1" || conv[16] || epoch:u32 || sender_device[16] || client_id[16] || class)
```

- `class` is the ASCII name: `post`, `edit`, `react`, `pin`, `meta`, `system`. The server sees it in clear and enforces its rules; the aad stops anyone from moving a body to another class.
- `class = meta` (meta rows and `conversations.meta_ct`) is sealed with `sender_device` and `client_id` set to the nil uuid (16 zero bytes), whatever the row says. The meta vector with nil ids is in `vectors/msg.json`.
- Padding: set `Envelope.padding` (field 15) to zero bytes so the encoded Envelope is a multiple of 256 bytes and `padding` is at least one byte (`msg::encode_padded`).
- Group meta (`meta_ct`) is an Envelope with a `Meta` body under the CK of `meta_epoch`, class `meta`.
- `system` rows are server-made plaintext JSON and carry no ciphertext (see System rows).

## System rows

`class = system`, `sender_device` empty, `body` is UTF-8 JSON (base64 on the wire like every body). The source of truth is `api/src/modules/chat/messages/system.rs` for conversations and `api/src/modules/chat/rooms/system.rs` for room chats. A tag this build does not know shows as a generic line.

Conversations (direct and group), tag `kind`, every row has `actor` (user id):

| `kind` | Fields |
|---|---|
| `created` | |
| `added` | `targets` (user ids) |
| `joined` | |
| `left` | |
| `kicked` | `target` |
| `banned` | `target` |
| `role` | `target`, `role` (`owner`, `admin`, `member`) |
| `renamed` | |
| `ttl` | `ttl_s` (seconds or null) |
| `deleted` | the row's `target_seq` is the deleted message, the row's `expires_at` is the target's |

`deleted` is delete for everyone: the target row comes back with no body, and clients drop it on every device.

Room chats, tag `type`:

| `type` | Fields |
|---|---|
| `join` | `user_id`, `name` |
| `leave` | `user_id`, `name` |
| `kick` | `user_id`, `name`, `by`, `ban` |
| `host` | `user_id`, `name` |
| `moderator` | `user_id`, `name`, `by`, `on` |
| `mute` | `user_id`, `name`, `by`, `until_ms` (null lifts it) |
| `title` | `by`, `title` |
| `track` | `urn`, `title`, `artist`, `artwork_url` |
| `suggest` | `user_id`, `name`, `track {urn, title, artist, artwork_url, duration_ms}` |

## Envelope

- `Text`: at most 4096 UTF-16 code units. `Span.start` and `Span.len` count UTF-16 code units. Kinds: bold, italic, code, spoiler, link (`url`), mention (`user_id`).
- `Edit {target, text}`, `Delete {target}`, `Reaction {target, emoji, on}`, `Pin {target, on}`, `Meta`, `Suggest {track}`.
- `reply_to`, `mentions[]`, `embeds[]` (3 at most), `forwarded_from`.
- `Embed` is a sender-made snapshot of a track, playlist, user or room invite; receivers validate it (https `*.sndcdn.com` artwork only, 3 KB at most) and render it without requests.
- `Attachment` field numbers are fixed now and filled in phase 2.
- A body this build does not know decodes to no body. Show "update the app to see this message".

Body kind to class:

| Body | Class | Server rule |
|---|---|---|
| `Text` | `post` | |
| `Edit` | `edit` | `target_seq` is a `post` of the same sender, at most 48 h old |
| `Reaction` | `react` | `target_seq` is a live `post` |
| `Pin` | `pin` | `target_seq` is a live `post`; admins only in groups and rooms |
| `Meta` | `meta` | groups only, admins only, nil ids in the aad |
| `Delete`, `Suggest` | none | never sent as an envelope: delete for everyone is `DELETE /chat/conversations/{id}/messages/{seq}?for=everyone`, a room suggestion is `POST /chat/rooms/{code}/suggest` |

## Vectors

| File | Covers |
|---|---|
| `request.json` | three requests (sync, history on pro, sealed login with a bind): keys, `ikm_e`, signature string, `Inner` bytes, envelope, `k_resp`, a sealed answer |
| `bind.json` | bind signature string, signature and protobuf bytes |
| `ck.json` | CK wrap under a KEK and CK seal to a device |
| `msg.json` | padded envelopes and message bodies for a text, a reaction, group meta and group meta under nil ids |

The keys in the vectors are test keys, never the pinned ones. `cargo test` replays every file (`tests/vectors.rs`).
