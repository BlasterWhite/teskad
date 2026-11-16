# The Hangar account (optional)

A Hangar account ("Compte Hangar") links several machines: sign in with GitHub
or Google on each, and the usage statistics of all of them can be read from any
of them.

It is optional and off by default. A Hangar nobody signed in with never calls
the account server, has no code path waiting on it, and works exactly as it
did before the account existed. Nothing in the app requires one, now or later.

Not to be confused with the *Claude accounts* a pane runs under
(`crates/hangar-core/src/accounts.rs`): those are Claude Code's logins, kept on
the PC, and have nothing to do with this.

## Pieces

| Piece | Where | What |
|---|---|---|
| Protocol types | `crates/hangar-cloud-api` | Requests, answers, paths, limits, `API_VERSION`. Serde only; both sides build from it. |
| Server | `crates/hangar-cloud` | One binary (axum, SQLite through rusqlite's `bundled`), one database file, a Docker image behind Cloudflare at `https://account.hangar.simnjs.fr`. |
| Desktop | `crates/hangar-core/src/cloud/` | Sign-in, the device token in the OS keychain, the sync. Behind hangar-core's `desktop` feature: a node has no browser to sign in with. |

The desktop reaches `https://account.hangar.simnjs.fr`, or the address in
`HANGAR_CLOUD_URL` when that variable is set (a local server, while developing).

## Privacy

### What leaves the PC

Nothing, until the person signs in.

At sign-in, once: the machine's name, its operating system (`windows`, `macos`
or `linux`) and an id the desktop drew at random for this install.

After that, **only while the statistics sync is on** (a setting of its own,
which signing in does not turn on by itself), totals per machine, day, model
and project name:

- the day (`YYYY-MM-DD`, in the machine's time zone), the model's id, and the
  project's **name**: the last component of its folder (`hangar` for
  `C:\Users\ada\Projects\hangar`);
- input, output, cache-read and cache-write tokens;
- what the API would have charged for them;
- active seconds, and the number of sessions.

Never a path, never a session's title, never a prompt, an answer or any other
line of a transcript. `StatsRow` has no field for them, and the server refuses
a project that holds a path separator, so a mistake on the desktop cannot send
one.

### What the server stores

- **The account**: an id, and what the provider said at sign-in: a display
  name, an email address when the provider gives a verified one, an avatar
  URL, and the provider's own id for the person. GitHub is asked for
  `read:user user:email`, Google for `openid email profile`. The provider's
  access token is used once, to read that profile, and is not kept.
- **The devices**: for each machine, the server's id, the machine's id, name
  and operating system, when it was linked and last seen, and the SHA-256 of
  its token. The token itself is sent once, at the exchange, and exists
  afterwards only in the machine's keychain.
- **The statistics rows** each device sent.
- **Sign-ins in progress**, for five minutes at most.

The server keeps no request log of its own and stores no IP address. Cloudflare,
in front of it, sees what any reverse proxy sees.

### Deleting

- **Signing out** (`DELETE /v1/devices/{id}` on oneself) destroys the device's
  token. Its rows stay on the account, so that signing in again from the same
  machine finds its history where it was.
- **Forgetting a machine** (`DELETE /v1/devices/{id}?forget=true`) deletes the
  device and every row it sent.
- **Deleting the account** (`DELETE /v1/account`) deletes everything: the
  account, its identities, its devices and their tokens, every row. At once,
  with nothing kept aside. The desktop then forgets its token too.

On the PC, the token is in the operating system's keychain (Credential Manager,
the macOS Keychain, the Secret Service), like the Groq key
(`crates/hangar-core/src/voice/key.rs`), and nowhere else. When no keychain
answers, sign-in fails and says so: the token is not written to a file instead.

## Signing in

The server runs the OAuth code flow with the provider; the desktop never sees
a provider's token or secret. Between the desktop and the server there is a
second, independent PKCE and state, so that only the Hangar that started a
sign-in can finish it.

```
desktop                          browser                 server                  provider
   |  listen on 127.0.0.1:<port>    |                       |                        |
   |  draw state + verifier         |                       |                        |
   |-- open /v1/auth/start -------->|---------------------->|                        |
   |                                |                       | store port, state,     |
   |                                |                       | challenge; draw its    |
   |                                |                       | own state + PKCE       |
   |                                |<-- 302 authorize -----|                        |
   |                                |------------------------------- consent ------->|
   |                                |<------------ 302 /v1/auth/callback/<provider> -|
   |                                |---------------------->| code -> token ->       |
   |                                |                       | profile (once)         |
   |                                |<-- 302 127.0.0.1 -----| draw link code         |
   |<- GET /callback?code&state ----|                       |                        |
   |  check state                   |                       |                        |
   |-- POST /v1/auth/exchange {code, verifier, machine} --->| check code, S256       |
   |<-- {token, account, device} ---------------------------|                        |
   |  token -> keychain             |                       |                        |
```

- The desktop's **verifier** is 43 to 128 characters of `[A-Za-z0-9-._~]`
  (RFC 7636), drawn from the OS's random source; the **challenge** is
  `base64url(sha256(verifier))` without padding. The **state** is random too,
  200 characters at most.
- The browser is only ever sent back to `http://127.0.0.1:<port>/callback`,
  with a port from 1024 up: the host and the path are the server's, never the
  request's.
- The **link code** is single-use and expires five minutes after the sign-in
  started. It is useless without the verifier, which never went through the
  browser.
- The **device token** is `hgr_` followed by 32 random bytes in base64url. It
  does not expire; it ends when the device signs out, is forgotten, or the
  account is deleted.
- **One device per machine.** The exchange carries the machine's id: when the
  account already has a device with that id, it is that device that is signed
  in again (its previous token stops working, its rows are kept) rather than a
  second one that appears.
- **One account per person.** An identity (a provider and its id for the
  person) belongs to one account. A first sign-in creates the account; a
  sign-in with the other provider whose *verified* email is the account's joins
  it, and `providers` then lists both.

## The protocol

Everything is under `/v1` (`API_VERSION`), over HTTPS, with JSON bodies in
UTF-8. `crates/hangar-cloud-api` is the reference: each request and answer
below is a type there, each path a constant of its `paths` module. Times are
milliseconds since the Unix epoch.

### Errors

Every answer that is not a success is

```json
{ "error": "invalid_code", "message": "the link code is unknown, used or expired" }
```

with the status of its code. `message` is English, for logs and bug reports;
the desktop words what it shows from `error`.

| `error` | Status | When |
|---|---|---|
| `bad_request` | 400 | A field missing or malformed, a name too long, a day that is not a day. |
| `invalid_code` | 400 | The link code is unknown, used or expired, or the verifier does not match. Which is not said. |
| `unauthorized` | 401 | No device token, or an unknown or revoked one. The desktop forgets its token and shows the account as signed out. |
| `not_found` | 404 | No such device on this account; no such route. |
| `method_not_allowed` | 405 | The route does not take this method. |
| `too_large` | 413 | More than 5000 rows, or a body over 4 MiB. |
| `rate_limited` | 429 | Too many requests; `Retry-After` says when to come back. |
| `internal` | 500 | The server's fault. |
| `provider_error` | 502 | GitHub or Google refused, or answered nonsense. |
| `provider_unavailable` | 503 | This server has no OAuth client for the provider. |

A desktop reads a code it does not know as a refusal like any other
(`ErrorCode::Unknown`).

### Without a token

#### `GET /health`

The one route outside `/v1`, for the proxy and the deployment.
`200 {"status": "ok", "api_version": 1, "version": "2.3.0"}`.

#### `GET /v1/auth/start`

Opened in the system browser, not called by the desktop.

| Query | |
|---|---|
| `provider` | `github` or `google` |
| `port` | The desktop's loopback port, 1024 to 65535 |
| `state` | The desktop's state, 1 to 200 characters |
| `challenge` | `base64url(sha256(verifier))`, 43 characters |
| `machine` | The machine's name, shown on the server's pages; 100 characters at most |

`302` to the provider's consent page. A query the server refuses, or a
provider it has no client for, is answered with the error (`400`, `503`): the
browser shows it, and the desktop's listener gives up after the sign-in's five
minutes.

#### `GET /v1/auth/callback/{provider}`

Where the provider sends the browser back: the redirect URI registered with
GitHub and Google. Not called by the desktop. `302` to
`http://127.0.0.1:<port>/callback` with

- `?code=<link code>&state=<desktop state>` when the person is signed in;
- `?error=access_denied&state=<desktop state>` when they said no at the
  provider, `?error=provider_error&state=…` for any other failure there.

A callback whose own state the server does not know (expired, replayed, forged)
is answered `400 bad_request` and redirects nowhere.

The desktop's listener answers the browser with a page saying the window can be
closed, checks `state` against its own, and stops listening.

#### `POST /v1/auth/exchange`

```json
{ "code": "…", "verifier": "…", "machine": { "id": "…", "name": "Atelier", "os": "windows" } }
```

`200`:

```json
{
  "token": "hgr_…",
  "account": { "id": "…", "display_name": "Ada", "email": "ada@example.org",
               "avatar_url": "https://…", "providers": ["github"] },
  "device": { "id": "…", "name": "Atelier", "os": "windows",
              "created_at": 1794000000000, "last_seen_at": 1794000000000 }
}
```

`email` and `avatar_url` are left out when the provider gave none. The code is
consumed by the first exchange that presents it, whether or not the verifier
matches. `400 invalid_code` otherwise; `400 bad_request` for a machine id, name
or `os` that is empty or over 100 characters.

### With a device token

`Authorization: Bearer <device token>` on every request; `401 unauthorized`
without a valid one. Each request moves the device's `last_seen_at`.

#### `GET /v1/me`

`200 {"account": {…}, "device": {…}}`: the account, and the device asking. What
the desktop calls at launch to learn whether it is still signed in.

#### `GET /v1/devices`

`200 {"devices": [{…}, …]}`: every device of the account, signed-out ones
included. A signed-out device carries `"revoked_at": <time>`; the field is
left out otherwise.

#### `PATCH /v1/devices/{id}`

`{"name": "Portable"}`, 1 to 100 characters. `200` with the device. Any device
of the account may rename any other. `404 not_found` for an id that is not the
account's.

#### `DELETE /v1/devices/{id}`

Signs the device out: its token stops working at once, its rows stay. With
`?forget=true` the device and its rows are deleted instead. `204`, with no
body. On oneself this is signing out, and the desktop then removes its token
from the keychain. Signing out a device that already is changes nothing (`204`);
`404 not_found` for an id that is not the account's.

#### `PUT /v1/stats`

```json
{ "rows": [ { "day": "2026-11-16", "model": "claude-opus-5-5", "project": "hangar",
              "input_tokens": 1200, "output_tokens": 3400,
              "cache_read_tokens": 560000, "cache_write_tokens": 78000,
              "cost_usd_micros": 1234567, "active_seconds": 5400, "sessions": 3 } ] }
```

The rows are those of the device whose token signs the request; a device
cannot write another's. Each row **replaces** the device's row of the same
(`day`, `model`, `project`): the values are totals, not increments, so sending
the same rows twice changes nothing, and a day still in progress is simply
sent again with its new totals. A key sent twice in one call keeps its last
row. Rows that are not sent are left as they are.

`200 {"upserted": <number of distinct keys>}`. The call is all or nothing:

- `413 too_large` over 5000 rows (the desktop sends more in several calls);
- `400 bad_request`, naming the first row at fault, when a `day` is not a
  calendar day, a `model` is empty or over 100 characters, a `project` is over
  100 characters (it may be empty) or holds `/` or `\`, either holds a control
  character, or a counter is over 2^63 − 1.

`cost_usd_micros` is the API-equivalent cost in millionths of a US dollar.
Lengths are counted in characters, not bytes.

#### `GET /v1/stats`

`?from=YYYY-MM-DD&to=YYYY-MM-DD`, both inclusive, either or both left out.

```json
{ "devices": [ { "id": "d1", "name": "Atelier", "os": "windows", "created_at": 1, "last_seen_at": 2 } ],
  "rows": [ { "device_id": "d1", "day": "2026-11-16", "model": "…", "project": "hangar", "…": 0 } ] }
```

Every row of the account in the range, from all its devices, the asking one
included, with the devices those rows name (signed-out ones too). A row is a
`PUT` row with `device_id` beside its fields. `400 bad_request` for a `from` or
a `to` that is not a day.

#### `DELETE /v1/account`

Deletes everything (see "Deleting"). `204`. Every token of the account,
this one included, is `401` from then on.

### Limits

| | |
|---|---|
| Rows per `PUT /v1/stats` | 5000 |
| `project` | 100 characters, may be empty, no `/` or `\` |
| `model` | 1 to 100 characters |
| Device name, machine id, `os` | 1 to 100 characters |
| `state` | 1 to 200 characters |
| Verifier / challenge | 43 to 128 / 43 characters |
| Counters | 0 to 2^63 − 1 |
| Request body | 4 MiB |
| Link code, and the sign-in it ends | 5 minutes |

They are constants of `hangar-cloud-api`, which the desktop applies before it
sends: a folder name over 100 characters is cut to 100.

### Versioning

`API_VERSION` is the `1` of `/v1`. Inside a version the protocol only grows:
new routes, new optional fields, new error codes. Both sides ignore fields
they do not know and neither may require a field the other did not always
send. Anything else (a field removed or renamed, a meaning changed) is a `/v2`
beside `/v1`, which the server keeps answering for as long as desktops that
speak it are in use. `GET /health` says which version a server speaks.

## Running the server

Configuration is the environment, read once at start
(`crates/hangar-cloud/src/config.rs`). A variable that is set and wrong stops
the server with a sentence naming it.

| Variable | Default | |
|---|---|---|
| `HANGAR_CLOUD_BIND` | `0.0.0.0:8080` | The address and port to listen on. |
| `HANGAR_CLOUD_DB` | `/data/hangar-cloud.sqlite` | The SQLite file. |
| `HANGAR_CLOUD_BASE_URL` | `https://account.hangar.simnjs.fr` | The server's public origin (scheme and host, no path): the OAuth redirect URIs are built from it. |
| `GITHUB_CLIENT_ID`, `GITHUB_CLIENT_SECRET` | none | The GitHub OAuth app. Both or neither. |
| `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET` | none | The Google OAuth client. Both or neither. |

A provider without its two variables is off: `/v1/auth/start` answers
`503 provider_unavailable` for it. The redirect URI to register with each
provider is `<HANGAR_CLOUD_BASE_URL>/v1/auth/callback/github` (or `/google`);
the server prints them at start.

The image is built from the root of the repository, where the workspace's
`Cargo.toml` and `Cargo.lock` are:

```sh
docker build -f crates/hangar-cloud/Dockerfile -t hangar-cloud .
docker run -d --name hangar-cloud -p 8080:8080 -v hangar-cloud-data:/data \
  -e GITHUB_CLIENT_ID=… -e GITHUB_CLIENT_SECRET=… \
  -e GOOGLE_CLIENT_ID=… -e GOOGLE_CLIENT_SECRET=… \
  hangar-cloud
```

It runs as a user of its own (uid 10001), listens on 8080 and keeps its one
file in the `/data` volume: that volume is the whole state of the service, and
what a backup copies. TLS is Cloudflare's, as for the site.

While developing, on a port of one's own:

```sh
HANGAR_CLOUD_BIND=127.0.0.1:21120 HANGAR_CLOUD_DB=./cloud.sqlite \
HANGAR_CLOUD_BASE_URL=http://127.0.0.1:21120 cargo run -p hangar-cloud
HANGAR_CLOUD_URL=http://127.0.0.1:21120 cargo run -p hangar-app
```

## Status

The foundation: this document, the protocol's types with their tests, the
server's configuration and `GET /health`, its Dockerfile, and the desktop
module's skeleton. The routes above, the sign-in on the desktop, the sync and
the settings page are built on it.
