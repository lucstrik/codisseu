# Native API backend (API-01, 2026-10-26)

Independent Rust implementation; no Postman code, icons, account client or runtime is incorporated.
Public API: `ApiService` owns the current `ApiWorkspace`, nonblocking profile jobs,
one active HTTP request/ordered collection run and a local mock listener. Profiles
are keyed by SHA-256 of the canonical project root, outside project sources.
Collection folders retain order; environment values override collection values.

HTTP uses reqwest 0.12.28/rustls on a dedicated Tokio worker. Only explicit Send or
Run collection starts requests. Timeout 100 ms–300 s; redirects at most 10;
response 1 KiB–16 MiB (default 4 MiB), request/upload 8 MiB, headers 256/64 KiB,
forms 4096 rows. Response cache: 8 entries; metadata history: 100; runner: 256
ordered requests. Cancel drops the HTTP future and kills/reaps a script child;
it cannot undo a request already received by the remote service. Retired jobs are
retained until the worker acknowledges completion. GUI polling never joins a
running thread. Listener is loopback only, ephemeral port, exact method/path
routes, 404 fallback, one client at a time, bounded reads. Mock stop also revokes
an asynchronous pending start.

Scripts are optional and require explicit opt-in. Runtime: installed Node.js
22.13+ with `--permission`, verified here with Node 24.22.0. The wrapper creates
all functions/data inside a `node:vm` context; no host object, require, process,
network, filesystem or package bindings are exposed. `vm` and Node permissions
are not claimed to provide a security boundary against malicious code. Use only
reviewed scripts. Code generation/wasm disabled, jitless, cleared environment,
temporary cwd, JS heap 64 MiB, VM timeout 1000 ms, host wall deadline 2500 ms and
bounded stdio. On Unix CPU 2 s and core dumps disabled; on Linux additionally
RLIMIT_DATA 256 MiB, including off-heap ArrayBuffer allocation. Child limits do
not bound the entire IDE. Other OS process memory behavior is not certified.
No runtime is automatically installed. Missing/unsupported Node fails the script
without applying its changes; HTTP without scripts works independently.

Compatibility subset: standard synchronous JavaScript, `pm.response.json/text`,
`code`, `responseTime`, `headers.get`, `to.have.status`, environment and collection
`get/set/has/unset/toObject`, `pm.variables.get/replaceIn`, `pm.test`, basic
`pm.expect` equality/deep equality/type/property/include/boolean assertions, and
bounded console log/warn. Environment set/unset requires a selected environment; pre-request failure prevents
the HTTP send, and post-response failure remains visible. Setting null/undefined
in either environment or collection scope fails without overwriting a valid token.
Each collection/folder/request source has an independent
lexical scope. No async `pm.sendRequest`, packages, full Chai/Postman sandbox,
OAuth token acquisition, AWS signing, gRPC, WebSocket, cloud mocks or monitors.
Runner applies environment/collection mutations before sending the next request;
script errors stop the run; failed assertions remain visible and do not become
successful tests. Changing selected environment aborts the remaining run.

Persistence excludes responses, histories, runner results, active mocks and
script-derived credentials. Secret rows and keys containing token/password/secret/
api_key are cleared in profile snapshots; literal Bearer/Basic password/API key
and sensitive headers are cleared unless a pure `{{variable}}` reference.
Source bodies/scripts/specifications and arbitrary custom fields may contain user
literals and are stored as source: use secret variables for credentials. Secrets
are session-only in this increment; no OS credential vault fallback is added.
Copying a resolved cURL intentionally puts the selected values on the clipboard.
Postman exports can be redacted by exporting `persistent_snapshot(workspace)`.

Atomic profile writes use a same-directory temporary file, sync and replacement;
Linux profile directory 0700/file0600. Invalid/schema-mismatched profiles cannot
be overwritten by save. Async save failures retain a nonsecret recovery snapshot
(max8 pending/recovery entries) for explicit retry. Filesystem atomicity and
cross-instance conflict handling are not universally certified. Load/save file
I/O never runs on the GUI thread.

cURL parsing never invokes a shell or performs variable/command expansion. Text
quotes/escapes, URL/query, headers, body/form, auth, timeout/TLS/redirect options
are parsed; unsupported options/mixed bodies/stdin/shell operators are errors.
Binary and multipart file references are imported without reading any file;
Send resolves relative paths against the project and reads only regular files,
with the 8 MiB total cap. Files outside the project require an explicit path in
the user-authored request. Multipart file modifiers and multiple files per field
are not supported. URL text edits and Params rows now share a query via `set_url`/`display_url`;
path variables are encoded by segment. Disabled rows and matched metadata survive URL edits.

Postman collection v2.1 and environments: nested folders, auth subset, variables,
headers/query, text/binary-file/form bodies and pre/post scripts. Unknown auth/body
modes are rejected. JSON-only collection/environment imports are bounded16 MiB.
OpenAPI 3.x JSON/YAML imports paths/operations/parameters/examples and one HTTP
Basic/Bearer/API-key security scheme; unsupported security is rejected. No
external refs are fetched; broad schema validation, full examples/defaults,
security alternatives and OAuth flows are not implemented.

Primary sources checked for this implementation:
- https://docs.rs/reqwest/0.12.28/reqwest/ (MIT OR Apache-2.0)
- https://docs.rs/url/2.5.8/url/ (MIT OR Apache-2.0)
- https://docs.rs/base64/0.22.1/base64/ (MIT OR Apache-2.0)
- https://docs.rs/sha2/0.10.9/sha2/ (MIT OR Apache-2.0)
- https://docs.rs/serde_yaml/0.9.34/serde_yaml/ (MIT OR Apache-2.0; upstream deprecated)
- https://docs.rs/nix/0.28.0/nix/sys/resource/fn.setrlimit.html (MIT)
- https://nodejs.org/docs/latest-v24.x/api/vm.html
- https://nodejs.org/docs/latest-v24.x/api/permissions.html
- https://schema.postman.com/json/collection/v2.1.0/collection.json
- https://spec.openapis.org/oas/v3.1.1.html

`cargo test -p codisseu-api --locked --jobs 2` exercises local HTTP, Node script
execution/resource failure, serial token propagation, imports/exports,
project persistence/isolation/recovery, cancellation and file uploads. This is a
backend test suite, not evidence of Postman account changes or native UI parity.

API-03 adds native folder editors and ordered tree operations (move validates
destination/depth before removing the source; duplicate regenerates descendant
IDs). New creation/duplication cap:10000 items in a workspace; folder depth32.
Existing JSON profile shape is preserved; path_variables/globals default empty.
Requests are boxed inside collection items to reduce enum footprint.

Scopes are now project globals → collection → environment → iteration data →
locals. Globals are project-specific and script-written values are secret in
persistent snapshots. Locals live for a Send or collection run; runner data is
temporary JSON objects, max1MiB/256rows. Runner accepts folders and explicit
iterations, max256 scheduled sends including conditional routing.
`setNextRequest` resolves ID then name within the selected subtree; null stops
that iteration. `skipRequest` produces a result flagged `script_result.skipped`
with no HTTP call/post scripts; internal response status0 is a sentinel, displayed
as Skipped, and runner result status is None. No response was received.

Additional scripting: pm.globals/variables local setters; iterationData getters;
pm.info fields/location; request header/query list access and mutation before
Send (request body immutable); response header/status/JSON assertions and common
negated/deep/numeric/length/regex Chai-like checks. These are independently
implemented, not full SDK/Chai; no async/network bindings or host objects added.
Timestamp/GUID/randomInt placeholders are test data, not cryptographic secrets.
OAuth2 import can reuse an explicit exported accessToken as Bearer; acquiring or
refreshing tokens and rewriting references to imported request IDs remain out of
scope. Public sources and concrete validation: docs/api-client-reference.md and
docs/status.md.
