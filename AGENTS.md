# Quietdeck

Quietdeck is source-available under FSL-1.1-ALv2 (`LICENSE.md`): each release becomes Apache 2.0 two years after
it's published. Call it "source-available" or "Fair Source", never "open source".

## Code from other projects

- **Apache 2.0 and MIT projects** (herdr, herdr-core, herdr-app): code can be borrowed. Keep their copyright notices
  and NOTICE file, and mark files you change.
- **GPL and AGPL projects** never go into Quietdeck, not even a snippet. `herdr-mobile-relay` (`~/code/herdr-mobile-relay`)
  is AGPL-3.0. Quietdeck's relay is our own design, not a port: when writing Quietdeck code, don't read anything under
  `~/code/herdr-mobile-relay`.
- Check a project's licence before taking anything from it, and ask Nathan about any other copyleft licence.

## Builds and tests run on docspring-dev

Rust builds, tests and clippy run on docspring-dev, the home server, never on a Mac: the workspace's `target/` grew
past 50 GB on Nathan's Mac and filled its disk. Use `scripts/on-dev test`, `scripts/on-dev clippy`,
`scripts/on-dev check` and `scripts/on-dev build` (cargo arguments go after the command). `test` copies back the
TypeScript that ts-rs writes to `web/src/types`. The web app (pnpm, Vite, Playwright) runs locally. The one Rust build
on a Mac is the dev deck's server, whose data stays there: `scripts/dev-deck-build` builds only that, without debug
info, in `~/.cache/quietdeck-dev-deck`.

Release images build on docspring-dev too: `scripts/build-images --wait <commit>` pushes
`harbor.home.ndbroadbent.com/quietdeck/deck:<sha>` and `node-bin:<sha>`, which the homeserver session deploys.
A Mac has no release binary: its node is built on the Mac itself with `scripts/install-node-from-source <commit>`
(release build only, in `~/.cache/quietdeck-node-build`, a temporary worktree), the one other Rust build allowed there.

## Adding an API route

The deck's API is published as an OpenAPI spec (`api/openapi.json`, served at `/api/v1/openapi.json`), generated
from the handlers by aide, so every route must be visible to it:

- Add it in `api.rs` (or `passkeys.rs`) with `.api_route("/path", post(handler))`, without the `/api` prefix (it's
  mounted at `/api/v1` and `/api`). A plain `.route` is invisible to the spec, and a test fails on one.
- Give it a line in `OPERATIONS` in `crates/quietdeck-server/src/openapi.rs`: operationId (the handler's name), tag,
  one-line summary; `ui("report", …)` for what only the default web app uses. A test fails until it has one.
- Path parameters are named structs: `Path(Id { id }): Path<Id>` (also `TextId`, `NodeName`). A bare `Path<i64>`
  drops the parameter from the spec. Query structs and body types derive `JsonSchema`.
- Within v1, change the API only additively (`api/README.md`); raise `API_VERSION`'s minor version when you add.
- Older nodes and CLIs stay in the field for days, and some types they read refuse unknown fields
  (`#[serde(deny_unknown_fields)]`: `SenderAuth`, `CardInput`, actions). A new field the deck sends back on one of
  them breaks every older CLI at once. On 28 November 2026 `sender_auth.spf_pass` stopped triage posting cards until
  its node was updated. Make such a field `Option` with `skip_serializing_if = "Option::is_none"`, and roll out a
  node-bin before anything sets it.
- Run `scripts/on-dev test`, which rewrites `api/openapi.json`, and commit it: CI fails when it differs from the code.
  Then `pnpm install && pnpm generate` in `clients/typescript` and commit `src/schema.ts` (and `API_VERSION` in
  `src/index.ts` when the spec's version moved): CI checks that too.

## Watching CI

Pushing main deploys the deck, and CI takes about 15 minutes. Watch it with
`timeout 1020 gh run watch <id> --interval 60 --exit-status`. Never use the default 3-second polling, and never loop
over `gh run list`. Every agent on Nathan's account shares one GitHub API limit (5,000 calls an hour). On 27 November
2026, several sessions watching at once used it up, and every session's `gh` failed for 35 minutes. When `gh` answers
403, `gh api rate_limit` says when it resets. Several sessions push to main, so a newer push cancels the run before
it: watch the newest run that contains your commit.

## Guides to write

Every time you run into something others will hit too, document it for the people who follow in our footsteps: a
setup step, a footgun (a prompt that blocks unattended sessions, a config that silently does nothing), a safety rule, a
design reason. This is a rule, not a nicety (Nathan, 25 November 2026).

1. Add an entry to `docs/guides-to-write.md` in the same change, saying what it is, how it was solved and where the
   knowledge lives now (a file, a commit, a runbook).
2. Send it to the planning session (`quietdeck msg send --to "planning@nathans-mac"`): the problem, the fix, how it
   was tested, and what's still open. Planning gathers it for Quietdeck's docs and for MailExec's, whose users run
   agents on their own machines too.

Do it when the problem is solved, not later: fast sessions move on, and nothing may fall through the cracks before the
guides are written.
