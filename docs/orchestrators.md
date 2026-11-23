# Orchestrators

An orchestrator is the program that runs your agent sessions in terminals: herdr's TUI is the one Quietdeck supports
today. Quietdeck's node talks to it through one interface, `Orchestrator` in
`crates/quietdeck-node/src/orchestrator/mod.rs`, so adding support for another orchestrator means writing one more
implementation of that trait. Nothing else changes: the deck, the CLI, the MCP tools and the database know nothing
about which orchestrator a machine runs.

This page is for people writing an adapter.

## The session address

Every session the orchestrator runs has an **address**: a string that only the orchestrator understands and that
the node gives back to it unchanged (for herdr, a pane ID such as `w1:p3`). The deck stores it with the session and
passes it to the node when a message must be typed into that session. Choose something that stays the same for the
session's lifetime and finds it again in one call.

Sessions also have a **tab** name, which people read and route by ("forward to **support**"). If your orchestrator
has no tabs, use whatever name a person gives the session.

## The interface

| Method | What it does |
| --- | --- |
| `name()` | A short name for logs and messages (`"herdr"`). |
| `sessions()` | Every agent session running on this machine, as `SessionInfo`: session ID, agent (Claude Code or Codex), tab, address, working directory, status and when it last changed. Set `open_url` to a link that brings the session forward on this machine, if your orchestrator has one, and `opens_in` to its name: the deck shows "Open in <name>". Leave out agents that don't have a session ID yet. An error means the orchestrator isn't running. |
| `subscribe()` | Its live events, as an `EventStream`. Return `StreamError::Refused` if it can't stream every session's events: the node then reads `sessions()` every 3 seconds instead, and tries the stream again every minute. Return `StreamError::Io` when it isn't reachable. |
| `open(tab)` | Open a new tab running an agent, without taking focus from the person: `OpenTab` has the label, an absolute working directory, the agent and the agent's own arguments (the node builds those: `--name`, `--resume`, the model). Answer once the agent takes input, with its address and session ID if the agent has reported it already. On failure, answer with a sentence the person can act on. |
| `close(session_id)` | Close the tab a running session is in. |
| `deliver(address, text)` | Type a message into a session as if the person had, and submit it. If the session isn't there, start the error with the node's `OFFLINE_PREFIX`, so the sender hears at once that it's offline. |
| `own_messages()` | Messages the orchestrator types into sessions by itself (herdr's auto-continue prompt after a restart). Summaries don't count them as the person's words. |

`EventStream::next()` returns the next `Event`, or `None` when the orchestrator closed the stream:

| Event | When |
| --- | --- |
| `Status { address, status }` | An agent's status changed: idle, working, blocked (waiting on the person) or done. |
| `TabClosed { sessions }` | A tab closed, with the sessions that were running in it. |
| `AgentExited { session, end }` | An agent's process exited; `end` says whether its terminal closed too. |
| `Updated { address, session, tab, cwd }` | A session's record changed (renamed, moved, its session ID arrived). |
| `Changed` | Something `sessions()` would show differently: the node reads it again. |
| `Other` | Anything else. Map unknown events here so a newer orchestrator doesn't break the node. |

Close events matter: they're how the deck tells a session someone closed from one that disappeared because the
orchestrator stopped or the machine went to sleep. Rules tied to a session are archived only on a real close.

## Telling the CLI where it runs

The `quietdeck` CLI, run by an agent inside a session, identifies its session to the node. It sends its Claude Code
session ID and process when it has them, and the address from the first environment variable it finds in
`SESSION_ADDRESS_ENV` (`crates/quietdeck-types/src/lib.rs`): `QUIETDECK_SESSION_ADDRESS`, then `HERDR_PANE_ID`. If
your orchestrator sets its own variable in each session's shell, add it to that list; otherwise set
`QUIETDECK_SESSION_ADDRESS`.

## Wiring it in

The node creates its orchestrator once at start (`run()` in `crates/quietdeck-node/src/main.rs`) and keeps it on
`Node.orchestrator`. The session watcher (`watch.rs`), message delivery (`deliver.rs`) and starting, resuming and
closing sessions (`control.rs`) all use it from there. herdr's implementation, `orchestrator/herdr.rs`, is the
example to follow: it shells out to herdr's CLI for one-off calls and reads herdr's event stream from its socket.

Not in the interface yet: reading a session's recent output, which a text view of sessions on the deck would need.
