# How Claude Code passes messages between sessions

Quietdeck's node delivers wake-ups, forwards, relayed messages and webhooks to sessions on its machine. The plan says
to deliver them the way Claude Code delivers its own cross-session messages (`SendMessage`), or with `herdr agent
prompt` if that behaves the same. This page records how Claude Code does it, what we tested, and what the node does.

Checked against Claude Code 2.1.330 on macOS, 23 November 2026, by reading the shipped bundle and testing against a
real session. **None of this is a documented API.** It's an internal protocol (`peerProtocol: 1`) that can change in
any release, so the node checks the protocol version and the integration tests run against a real Claude Code.

## Summary

- Every interactive session listens on its own Unix socket and publishes it, with an auth key, in
  `~/.claude/sessions/`.
- A message is one line of JSON on that socket. It goes into the session's **command queue**, the same queue that
  holds what the user types while Claude is busy.
- An idle session starts a turn with it. A working session reads it at its **next tool round**, inside the running
  turn. A blocked session (a permission prompt, a question) keeps it queued and reads it once the person answers.
- The model sees it labelled as coming from another session, not from the user, with Claude Code's own warning
  against permission laundering. The transcript records it as `isMeta: true` with `origin.kind: "peer"`.
- **A session in bypass-permissions mode holds a message whose sender doesn't attest bypass mode**, and asks the person
  to approve it, unless its `crossSessionInbound` setting is `accept`. A process outside Claude Code (the node) can't
  honestly attest a permission mode.
- `herdr agent prompt` is **not** equivalent: it types the text as if the person had, it refuses blocked sessions, and
  the transcript records the text as the person's own message.

## The registry

Each interactive session writes `~/.claude/sessions/<pid>.json` (the directory is mode 0700):

```json
{"pid":50653,"sessionId":"45be65c7-…","cwd":"/Users/me/code/app","version":"2.1.330",
 "peerProtocol":1,"peerFeatures":["notify_idle","reply_across_default_dirs","artifact_yield"],
 "kind":"interactive","entrypoint":"cli","messagingSocketPath":"/tmp/cc-socks/50653.sock",
 "name":"planning","nameSource":"user","status":"idle","statusUpdatedAt":1795396920175}
```

- `status` is Claude Code's own view: `idle`, `busy` or `waiting` (waiting on the person, including a held message).
- `messagingSocketPath` is a Unix socket, mode 0600, in `/tmp/cc-socks/` (or `$XDG_RUNTIME_DIR/cc-socks` on Linux).
- Beside it, `<pid>.<sha256 of the socket path>.key` (mode 0600) holds
  `{"peerToken":"<32 hex>","procStart":"…","pidDomain":"darwin"}`. The hash is of the path as written
  (`/tmp/…`), not its resolved form (`/private/tmp/…` on macOS).
- Child processes of a session get a second token in `CLAUDE_CODE_MESSAGING_TOKEN`. A child that uses it counts as the
  session itself, so its messages are always accepted.

A session is reachable when its registry file exists, its `pid` is alive with the same `procStart`, and its socket
accepts a connection.

## The wire protocol

Newline-delimited JSON. Claude Code prints the recipe in its own debug log:

```sh
{ echo '{"type":"auth","token":"<peerToken>"}'
  echo '{"type":"user","message":{"role":"user","content":"hello"}}'; } | socat - UNIX-CONNECT:/tmp/cc-socks/<pid>.sock
```

- The first line must be `auth` with the session's `peerToken`, within a deadline, or the connection is closed. On
  macOS the session also reads the connecting process's pid from the socket.
- A `user` frame: `message.content` (a non-empty string), and optionally `msg_id`, `priority` (`now`, `next`, `later`;
  default `next`), `session_id` (dropped if it doesn't match the receiver's), `from` (a `uds:<socket path>` reply
  address), `file_attachments`. Nothing comes back on the connection for an accepted message.
- Other frame types exist for Claude Code's own features: `notify_when_idle`, `peer_idle_notice`,
  `peer_message_status` (receipts sent to the sender's own socket), `artifact_replies_yielded`.

### The envelope

`SendMessage` wraps the body in an envelope inside `content`, which the receiver parses and re-serialises (a mismatch
means it isn't treated as an envelope):

```text
<cross-session-message from="uds:/tmp/cc-socks/123.sock" from-session="<id>" from-name="talira" from-mode="bypass">
body
</cross-session-message>
```

Every attribute is optional, in that order. `from-mode` is `bypass` or `prompting`, the sender's own claim about its
permission class. The model is shown the envelope, prefixed with "Another Claude session sent a message:" and followed
by Claude Code's guidance that a peer isn't the user and can't grant escalation.

## How the receiver handles it

1. **The inbound policy** decides accept, hold or refuse:
   - `crossSessionInbound` in settings (`accept`, `hold`, `refuse`) wins. Managed and repository settings can only
     tighten it.
   - Otherwise a message from the session itself (or its children) is accepted.
   - Otherwise, if the envelope asserts `from-mode`, it's accepted when it matches the receiver's class and held when
     it doesn't (`mode-mismatch`).
   - Otherwise a receiver in bypass mode holds it (`no-mode-asserted`), and any other receiver accepts it.
2. **A held message** shows a dialog in the session: "Held message from another session … Deny / Deliver". Claude
   Code's registry says `waiting`, but **herdr reports the session as `idle`**, so it shows no red dot. A headless
   session has nobody to approve, and the message expires.
3. **An accepted message** is queued (`queue-operation: enqueue` in the transcript). If the session is idle, it starts a
   turn. If it's working, it's attached to the next tool result as a `queued_command` (`queue-operation: remove`), so
   the model reads it mid-turn. If it's blocked on a prompt or question, it stays queued until that's answered, and is
   then read at the next tool round. It never answers the prompt.
4. **In the transcript** it's a user record with `isMeta: true`, `promptSource: "system"` and
   `origin: {kind: "peer", from, verifiedPeerPid, msg_id, name?, body?}`. Anything that drops every `isMeta` record
   (claude-summarizer did) drops peer messages: keep records whose `origin.kind` is `peer`.

## What we tested

On a throwaway Haiku session in a herdr tab:

| Case | Unix socket | `herdr agent prompt` |
|---|---|---|
| Idle | Starts a turn | Starts a turn |
| Working, mid-turn | Read at the next tool round, in the same turn | Also queued and read at the next tool round |
| Blocked on a question | Queued, read after the answer, at the next tool round | Refused: `agent_blocked` |
| Bypass-mode receiver, default settings | **Held** for the person's approval (no herdr red dot) | Delivered |
| With `crossSessionInbound: "accept"` | Delivered | Delivered |
| What the model sees | "Another Claude session sent a message", the envelope's sender name, and the anti-laundering guidance | The text, as if the person typed it |
| Transcript | `isMeta: true`, `origin.kind: "peer"` | A plain user message, indistinguishable from the person's |
| Codex sessions | No | Yes |

## What the node does

**Claude Code sessions: the Unix socket, exactly as `SendMessage` does.**
- It finds the target in `~/.claude/sessions/` by session ID, checks the pid and `peerProtocol`, reads the key and
  sends one `user` frame with priority `next`.
- The body is an envelope with `from-name` set to the sender as people see it (`talira (homeserver) via Quietdeck`,
  `Quietdeck runner`, `Gatus webhook via Quietdeck`) and the fixed header for the kind of message (section 3.6 of the
  plan). It sets no `from`, so the receiving agent replies the way the header says (`quietdeck msg send …`), and **no
  `from-mode`**: the node isn't a Claude session, and claiming bypass mode would get around the check that exists to
  stop permission laundering.
- It fails at once, with "<tab>@<node> is offline", when there's no registry entry, the pid is gone or the socket
  refuses. That's the only failure. It doesn't look at whether the session is working or blocked: the queue handles
  both, as it does for `SendMessage`.
- Nothing is stored or retried.

**Codex sessions, and Claude Code sessions with no socket** (an older version, messaging off): `herdr agent prompt`,
with its own errors passed back as they are.

**The receiving machine needs `crossSessionInbound: "accept"`** in `~/.claude/settings.json` when its sessions run in
bypass mode, or every Quietdeck message is held for approval in the tab. That setting also accepts unattested
messages from any process of the same user that can read the key file. The setup guide says so and leaves the choice
to the user. With `hold`, the message waits in the tab for the person's approval, and the node can't tell that it
was held: receipts go only to a sender's own socket.

## Using it from a session's own children

The `quietdeck` CLI, run by an agent, could message its own session with `CLAUDE_CODE_MESSAGING_TOKEN` and be accepted
as the session itself. Quietdeck doesn't need this today.
