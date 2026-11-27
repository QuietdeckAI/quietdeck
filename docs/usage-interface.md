# The usage module's interface

The node's usage module reads this machine's Codex subscription. The node pushes its readings to the deck and serves
them to the menu bar app. Claude sign-ins, their usage and the account rotator's coordinator API are Fleetlayer's
claude-auth-manager plugin's: the node never reads a Claude sign-in, takes `coordinator.lock` or binds
`coordinator.sock`. This page is the contract between the
node's core and the module.

- **Records:** `crates/quietdeck-types/src/usage.rs`. TypeScript copies are generated into `web/src/types/`.
- **Entry points:** `crates/quietdeck-node/src/usage/mod.rs` (`spawn`, `UsageHandle`, `UsageEnv`, `UsageEvent`).

## What the node passes in

| Input | Type | Notes |
|---|---|---|
| Settings | `UsageConfig` | From `usage:` in node.yaml. Defaults: poll every 5 minutes at most, Codex on. The old `claude` and `serve_coordinator_api` keys are accepted and ignored |
| Machine facts | `UsageEnv` | The home directory |
| Commands | `UsageHandle::command(UsageCommand)` | `check` re-reads Codex; `sign_in` is refused (Codex signs in with `codex login`, Claude through Fleetlayer) |

## What the module hands back

`mpsc::Sender<UsageEvent>`. Each `Snapshot` is every subscription on the machine. The node sends it to the deck as
`UsagePush { node, accounts, at }` (dropping repeats it has already sent) and keeps the latest for the menu bar app.

`AccountUsage` carries no token or secret. `account_key` must be the same for one subscription on every machine
(OpenAI: the ChatGPT account ID), because the deck
merges on it. `local_id` is the machine's own ID, which commands use.

Windows: `codex_primary` / `codex_secondary` with `window_minutes` for Codex. `pct` is percent used as last read.

## What the module owns

Reading the Codex sign-in (`~/.codex/auth.json`), polling its usage endpoint, and the readings Codex writes into its
session logs. The summaries module reads its "soonest reset" Claude account from Fleetlayer's `coordinator.sock`.

## Changing the contract

Change the types in `quietdeck-types` and this page in the same commit. `cargo test` regenerates the TypeScript.
