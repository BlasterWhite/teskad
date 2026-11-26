# Codex app-server protocol types

Hangar talks to Codex through `codex app-server`, a JSON-RPC protocol: attached panes and headless
children (docs/native/agents.md §4.1, §4.3, decision D10). Its Rust types are not written by hand:
they are generated from the JSON Schema the pinned Codex release prints itself
(`codex app-server generate-json-schema`), reduced to the methods Hangar uses, and checked in. They
are regenerated when the pin moves, and never edited.

| File | What it is |
|---|---|
| `subset.json` | The pin (`"codex"`), whether the experimental surface is included (`false`: Hangar sends `experimentalApi: false`), and the methods Hangar uses: requests it sends, notifications and requests it reads. `responseOverrides` names an answer type Codex's `…Params` → `…Response` naming does not give. |
| `fetch-schema.sh` | Installs the pinned Codex from npm into a throwaway prefix, runs its `codex app-server generate-json-schema` into a new temporary folder and prints its path. Nothing is installed globally. |
| `generate.mjs` | Reduces the schema to the subset and writes the outputs below. Node only, no dependencies; needs `rustfmt`. |
| `diff-schema.mjs` | The schema watch's diff of two Codex releases' schemas, restricted to the subset (below). Node only, no dependencies; its reduction is `generate.mjs`'s. Tested by `diff-schema.test.mjs` (`node --test scripts/codex-protocol/`). |
| `schema.subset.json` (generated) | The schema reduced to the subset's methods and every type they reach: what a bump's review reads, and what `generate.mjs` alone regenerates from. |
| `crates/hangar-core/src/codex_app/generated.rs` (generated) | The Rust types, `serde`. |
| `crates/hangar-core/src/codex_app/testdata/schema-samples.jsonl` (generated) | A JSON instance of every type, variant and value, written from the schema alone; `codex_app/tests.rs` reads each back with its Rust type. |

## What the generated code is

- One Rust type per schema type of the subset, with the schema's names and its descriptions as doc
  comments; a field is `snake_case` with its wire name in `#[serde(rename)]`. Optional fields are
  `Option`s, left out of what Hangar sends when `None`. A struct whose fields allow it derives
  `Default`, so params are written `ThreadStartParams { cwd: Some(…), ..Default::default() }`.
- Unions follow the schema: a property with one value per alternative is `#[serde(tag = …)]`
  (`ThreadItem` by `type`), strings and single-key objects are serde's default representation
  (`AskForApproval`), anything else is tried in order (`#[serde(untagged)]`, its variants named
  after their types less the words they share: `McpElicitationPrimitiveSchema::Number`).
- `requests::*`, `notifications::*` and `server_requests::*` are marker types: `ClientRequest`
  gives a request's `METHOD`, `Params` and `Response`; `ServerRequestMethod` the answer a server
  request waits for. `ServerNotification` and `ServerRequest` read a whole JSON-RPC message by its
  `method` (other keys, such as `id`, are left to the client).
- `CODEX_VERSION` is the pin.

## Lenient, so a newer Codex does not break Hangar

- Unknown fields are ignored (no `deny_unknown_fields`).
- Every string enum ends with `Unrecognized(String)`, even one of a single value today
  (`ModelRerouteReason`): Codex adds reasons and kinds. Every union ends with
  `Unrecognized(serde_json::Value)`, which keeps what came; a variant that cannot be read falls
  there too.
- `ServerNotification::Unrecognized` and `ServerRequest::Unrecognized` hold the whole message: a
  method outside the subset (a request then gets `methodNotFound`), or one of the subset this
  version could not read, which `METHODS.contains(&message.method())` tells apart, so the client
  can report it once.
- A field the schema requires stays required: a message without it is not silently filled in.
- Two documented exceptions, needed to tell shapes apart: an enum only ever used as an object's
  `type` stays exact (MCP elicitation's `"string"`, `"number"`…), and an object the schema closes
  (`additionalProperties: false`) refuses unknown fields when it is one shape of an untagged union,
  as Codex's own decoder does. Neither fails a message: what matches no shape is the union's
  `Unrecognized`. An untagged union inside another goes without a fallback of its own, or it would
  swallow the shapes after it.
- An object with alternative shapes of its own (the image item's `fileId` or `result`) takes all
  their fields as `Option`s.

## How to bump the pinned Codex version

1. Pick the release: `npm view @openai/codex version` (or the GitHub releases of openai/codex) and
   read its release notes for app-server changes.
2. Set `"codex"` in `subset.json` to it.
3. Fetch its schema and regenerate:

   ```sh
   scripts/codex-protocol/fetch-schema.sh --generate
   ```

   It installs that Codex from npm into a throwaway prefix, removed on exit (install scripts off,
   no login, an empty `CODEX_HOME`), runs `codex app-server generate-json-schema` into a new
   temporary folder, checks the version it printed and runs `generate.mjs --schema <folder>` on it.
   It needs node, npm and the network; no Codex of yours is used or changed.
4. Read what changed for Hangar: `git diff scripts/codex-protocol/schema.subset.json` (the protocol
   itself) and `git diff crates/hangar-core/src/codex_app/generated.rs` (the types).
5. When the generator stops, it says why:
   - a method of `subset.json` is gone or renamed → follow the release notes, change the list;
   - no response type for a method → add it to `responseOverrides`;
   - a type name is two different types (root and v2 namespaces) → name one in `flattenBundle`;
   - a union of a new shape → teach `Generator.union` that shape.
6. Build and test:

   ```sh
   cargo test -p hangar-core --lib codex_app
   scripts/ci-check   # the whole CI on GitHub Actions, for the committed HEAD
   ```

   `every_shape_the_schema_allows_reads_and_writes_back_unchanged` reads every sample of the new
   schema; a failure names the type and the case. Then fix the code that uses the types (the client
   in `codex_app/`, `agent/codex*.rs`).
7. The release the recorded fixtures pass on is `CODEX_TESTED` in `agent/codex.rs` (§4.6): move it
   with the fixtures, not with this pin alone.
8. Commit `subset.json`, `schema.subset.json`, `generated.rs` and `schema-samples.jsonl` together.

## Other changes

- **A method more (or less)**: edit the lists of `subset.json`, then `fetch-schema.sh --generate`:
  the reduced schema only has the types of the methods it was written for, so `generate.mjs` alone
  refuses.
- **A generator change, same pin**: `node scripts/codex-protocol/generate.mjs` rewrites the outputs
  from `schema.subset.json`; no Codex needed.
- **Is everything up to date?** `node scripts/codex-protocol/generate.mjs --check` writes nothing
  and exits 1 when a generated file is not what the generator writes.
- **An experimental method**: set `"experimental": true` (the schema then includes the gated
  methods and fields) and have the client send `experimentalApi: true`; record why in the commit.

## Schema watch

Codex ships about three releases a week, and the pin only moves by hand. So that Hangar learns when a
release changes what it uses, `.github/workflows/codex-schema-watch.yml` runs every Monday (05:23 UTC)
and on demand, on a GitHub runner: no secret, no login.

1. It reads the latest release (`npm view @openai/codex version`) and stops there when it is not newer
   than the pin.
2. It installs the pinned release and the latest one from npm, each into a throwaway prefix (install
   scripts off), and runs `codex app-server generate-json-schema` on both with an empty `CODEX_HOME`.
3. `diff-schema.mjs` compares the two schemas restricted to the subset: both go through
   `generate.mjs`'s reduction (the methods of `subset.json` and every type they reach), so what is
   compared is what the generated types come from. It prints the methods of the subset that are gone,
   and per method and type the fields added, removed, now required or no longer, the changed types and
   the enum values and alternatives added or removed. Descriptions are not compared. Methods outside
   the subset are listed for information and never count.
4. When the subset changed it opens an issue labelled `codex-schema` with the diff and the release's
   link, or updates the one already open (a comment says when a newer release replaced the one it was
   about): never two. Unchanged, it does nothing. It never fails a release: it is a notice, in a
   workflow of its own. It fails only when the diff cannot be made, which shows in Actions.

What to do with the issue: read the release notes against the diff, then bump the pin (above), or,
when the release drops something Hangar needs, list it in `CODEX_BROKEN`
(`crates/hangar-core/src/codex_app/gate.rs`) so Hangar runs the plain TUI for it meanwhile. Closing the
issue is by hand.

**By hand.** Either let the workflow do it for any release
(`gh workflow run codex-schema-watch.yml -f version=0.171.0`, or Actions > Run workflow), or compare
two schema folders yourself: `fetch-schema.sh` writes the pinned release's, and a Codex of the other
release writes its own:

```sh
codex app-server generate-json-schema --out <folder>   # CODEX_HOME empty, no login
codex --version > <folder>/codex-version.txt           # which release wrote it
node scripts/codex-protocol/diff-schema.mjs <pinned folder> <latest folder>
```

The pinned folder must be the release `subset.json` pins. Exit 0: the subset is unchanged; 1: it
changed (the diff is on stdout); 2: the diff could not be made. `--subset <file>` reads another
subset than `subset.json`.
