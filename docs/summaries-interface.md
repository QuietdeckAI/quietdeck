# The summaries module's interface

The node's summaries module writes every session's summaries on its machine. The node pushes them to the deck, which
stores them and serves the Sessions screen, search, `quietdeck sessions` and `who-has`. This page is the contract
between the node's core and the module.

- **Records:** `crates/quietdeck-types/src/summary.rs`. TypeScript copies are generated into `web/src/types/`.
- **Entry points:** `crates/quietdeck-node/src/summaries/mod.rs` (`spawn`, `SummariesHandle`, `SummariesEnv`).

## What the node passes in

| Input | Type | Notes |
|---|---|---|
| Settings | `SummariesConfig` | From `summaries:` in node.yaml. Defaults: idle 60 s, gap 30 min, 2 calls at once, 48,000-character cap, day chunks for 30 days, sweep on, model `sonnet` |
| Machine facts | `SummariesEnv` | State directory, home, the `claude` binary, the coordinator socket (or none), the time zone |
| Session signals | `mpsc::Receiver<SessionSignal>` | One `Status` for every session in herdr's snapshot when the node starts, then one per status event from the orchestrator, and a `Closed` per close event |
| Catch-up requests | `SummariesHandle::catchup(CatchupRequest)` | When the chat view opens a session |
| The import | `SummariesHandle::import_claude_summarizer(dir)` | Run once per machine, from `quietdeck-node import-summaries <dir>` |

`SessionFacts` carries what herdr knows: session ID, agent, tab label, cwd, and the transcript path when the node has
resolved it. The module finds the transcript itself otherwise.

`done` (idle, but nobody has looked) counts as idle. `blocked` never does: a blocked session is never summarised.

## What the module hands back

`mpsc::Sender<SummaryEvent>`. The node wraps each event as `SummaryPush { node, event }` and sends it to the deck over
its WebSocket. Pushes that can't be sent while the deck is unreachable are kept in order and sent on reconnect, so the
module never retries a push itself.

- `Summary(SummaryRecord)`: a new or replaced timeline point. Its key is `session_id` plus `period`. Live summaries use
  `period: {kind: "hour", key: "2026-11-23T14"}`; old history uses `day` and `week`.
- `Activity { session_id, last_activity }`: after every successful call, `no_change` included.
- `Catchup(Catchup)`: a regenerated catch-up. `catchup()` also returns one directly.

## What the module owns

Transcript discovery and byte offsets, the noise filter, the "worth a call" threshold, `no_change`, the idle timer
and its cancellation, the 30-minute gap (an idle inside it is scheduled for when it ends, never dropped), one pending
summary per session, at most `max_concurrent` calls, the start-up and daily sweeps, day and week chunks, the cap and
its trimming order, the skips (subagents, `codex exec` reviews, headless runs with an entrypoint other than `cli`),
skipping a chunk that fails three times, choosing the account, running headless Claude Code, and its own state under
`state_dir`. Offsets move only after a successful call.

## Changing the contract

Change the types in `quietdeck-types` and this page in the same commit. `cargo test` regenerates the TypeScript.
