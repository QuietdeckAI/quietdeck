# The usage module's interface

The node's usage module takes over the account rotator coordinator's usage monitoring and adds Codex. The node pushes
its readings to the deck (the Usage screen) and serves them to the menu bar app. This page is the contract between the
node's core and the module.

- **Records:** `crates/quietdeck-types/src/usage.rs`. TypeScript copies are generated into `web/src/types/`.
- **Entry points:** `crates/quietdeck-node/src/usage/mod.rs` (`spawn`, `UsageHandle`, `UsageEnv`, `UsageEvent`).

## What the node passes in

| Input | Type | Notes |
|---|---|---|
| Settings | `UsageConfig` | From `usage:` in node.yaml. Defaults: poll every 5 minutes at most, Codex on, coordinator API on |
| Machine facts | `UsageEnv` | The rotator's directory, home, the `claude` binary, the module's state directory |
| Commands | `UsageHandle::command(UsageCommand)` | `sign_in` and `check`, from the Usage screen and the menu bar app |

## What the module hands back

`mpsc::Sender<UsageEvent>`. Each `Snapshot` is every subscription on the machine. The node sends it to the deck as
`UsagePush { node, accounts, at }` (dropping repeats it has already sent) and keeps the latest for the menu bar app.

`AccountUsage` carries no token or secret. `account_key` must be the same for one subscription on every machine
(Anthropic: `<account uuid>/<org uuid>` from the profile endpoint; OpenAI: the ChatGPT account ID), because the deck
merges on it. `local_id` is the machine's own ID, which commands use.

Windows: `five_hour` and `week` for Claude, `model_week` with `model` for a model family's own week, and
`codex_primary` / `codex_secondary` with `window_minutes` for Codex. `pct` is percent used as last read.

## What the module owns

Reading each sign-in, polling the usage endpoint, readings that sessions post, keeping idle sign-ins fresh with
`claude -p /usage`, browser sign-ins and warnings, model weeks, Codex usage, the session-to-account map, and serving
the coordinator API (`docs/coordinator-api.md` in the rotator) on `<rotator_dir>/coordinator.sock` while
`serve_coordinator_api` is on. The summaries module reads its "soonest reset" account from that same socket.

## Changing the contract

Change the types in `quietdeck-types` and this page in the same commit. `cargo test` regenerates the TypeScript.
