# Guides to write

Nathan (25 November 2026): "we're implementing so much stuff and solving so many problems in quick sessions, and almost
all of it is needed by everyone to build a robust and secure cluster of AI agents." Each entry below is something we
solved or decided for ourselves that belongs in Quietdeck's published guides. **Add an entry in the same change that
solves the problem**, with where the knowledge lives now, so nothing is lost before the guides are written. Strike an
entry through only when its guide is in `docs/`.

Drafts other sessions wrote for these are in `docs/drafts/`.

## Running agents

- **Moving a session to another machine.** Handoffs, the access manifest, `handoff claim`, closing the old session.
  Now: `skills/move-session/SKILL.md`, and the quietdeck move (h_2, 24 November).
- **Agents addressed by role, not tab name.** Why a role setting beats a tab name hard-coded across runner configs,
  lease fallbacks, instructions and skills; lightweight agents any session can take on. Now: build plan "Agents are
  addressed by role, not tab name (25 November)". Not built yet.
- **Monitoring the sessions that must stay up.** Long-running agents with responsibilities (triage, infrastructure,
  a queue worker) are marked monitored, and the node pings a heartbeat URL while each runs, so Gatus, Healthchecks or
  Cronitor alerts when one stops; the rest can be closed any time. Agents propose it on a card and the person
  approves. Now: build plan (Nathan, 25 November). Not built yet.
- **Sessions survive a reboot.** herdr resumes Claude Code sessions after a VM restart, with their conversation;
  background processes and the session's temporary scratchpad don't survive. Now: the agents VM restart, 25 November.
- **Per-machine instructions.** A shared global instructions file for every machine, plus one file per machine
  (`CLAUDE.this-machine.md`, which the sync leaves out) for what holds only there: no browser, no Touch ID, how to
  reach the other machines. Now: Nathan's global instructions and the homeserver repo's agents VM role.
- **Agents propose, the person approves.** Subscriptions, monitoring, rules and new sessions are proposed by agents
  on cards and approved by the person, never written into a config file the person has to edit. Now: Nathan, 25
  November (build plan).
- **Two scratchpads per agent.** A wiped one (`/tmp`, the session scratchpad) and a kept, synced one in the notes
  vault, named after the role. Now: Nathan's global instructions, "Agents' scratchpads: one wiped, one kept".
- **Builds off the laptop.** Heavy Rust builds and tests on a build server (`scripts/on-dev`), heavy-build leases, why
  a laptop's disk fills. Now: `AGENTS.md`, `docs/leases.md`.

## The triage agent

- **The triage agent on its own Unix user and node.** It reads untrusted mail with permissions skipped, so no
  credentials or repos may share its user; its own node and herdr; how other sessions reach it. Now:
  `docs/drafts/homeserver-guides.md`, the homeserver repo's agents VM role.
- **While you read the Inbox, nothing new lands.** Presence, held wake-ups, "while I'm reading" rules (RE2), toasts,
  Approve All's ten-second settle window. Now: `docs/getting-started.md` (a paragraph), commit 3b8de63. Needs a full
  guide with the reasoning: a page that changes under the person is how a wrong approval happens.
- **Archiving in Gmail yourself keeps the report in step.** Mail archived or deleted on a phone or in the browser
  leaves the inbox, the runner confirms each thread has really gone (every message, not one), the deck resolves the
  cards whose threads have all left, and the agent tidies the report, or marks it done with `quietdeck report done` so the
  Inbox says "All done." The history tool needs `labelRemoved` and `messageDeleted`. Now: `deploy/node.example.yaml`,
  `crates/quietdeck-server/src/mail.rs` (`threads_left`), the email-triage skill (25 November).
- **What plain code may read.** The runner reads headers, labels and snippets but never logs them; plain code is the
  safe place to read untrusted text (no prompt injection), so "never read content" isn't a rule worth keeping.
  Now: this conversation (25 November), `crates/quietdeck-node/src/runner/mail.rs`.

## Inputs beyond email

- **Matrix as a message source.** A service user, an end-to-end encrypted room, the device and crypto store kept on
  one node, `quietdeck messages`, `quietdeck-node matrix-check`, cards with account `matrix`. Now: sc-7779,
  `deploy/node.example.yaml`, commits ec81b71 to 23de605.
- **Alerts forwarded to the agent that owns the service.** Subscriptions from a channel to a role, written by agents
  and approved by the person; retries and a fallback to triage. Now: the design sent to planning (25 November). Not
  built yet.

## Watching everything in the background

- **Every background thing has a check that alerts, and an agent listens.** Heartbeats for scheduled jobs, health
  checks for daemons, checks that catch "running but not working", alerts to a Matrix room, and the owning agent
  fixing it. Now: Nathan's global instructions, "Everything in the background is watched, and an agent fixes it".
- **A dead man's switch for the alert path.** Alertmanager's Watchdog posts to its own Matrix room every 30 minutes;
  the node's Matrix reader turns each post into a ping to Healthchecks.io (plain code, no card, no agent, no tokens),
  so if Prometheus, Alertmanager, the bridge, Synapse or the reader dies, the silence emails the person. Now:
  `deploy/node.example.yaml` (`matrix.heartbeats`), Nathan's approval on 25 November.
- **Health endpoints for Quietdeck's own parts.** Each node, the runner and the Matrix reader, for Gatus; alerting
  that doesn't depend on Quietdeck when Quietdeck is the thing that's down. Now: the design sent to planning (25
  November). Not built yet.
- **A Machines view.** Which machines run, what each is, up or down and last restart, which agents run where, load and
  disk per session and build, announced restarts. Now: build plan "A Machines view on the deck"; System design
  "Machines and internal notifications (23 November)". Not built yet.

## Security and access

- **Zones of trust and a network map.** Which agents are trusted alike, the flows between zones, generating the map
  from the repos, Tailscale ACL tests. Now: Notes, "Security and access".
- **Access levels: none, read-only, read-write.** For file mounts, Quietdeck's messaging and sessions, and services;
  spaces, principals and roles. Now: Notes, "Security and access". Waiting on Nathan's decision; a draft is parked on
  branch `acl-v1` (local only).
- **Secrets the person pastes in.** Create the stub 1Password item first, then name it. Now: Nathan's global
  instructions. `docs/secrets-management.md` covers the rest.

- **Proposals under discussion** (nothing decided; don't build from them): one front door for outside text (P1), a
  coordination log of who asked whom (P3), gates on outward actions (P4). Now: the proposals page,
  https://rag.docspring.io/artifacts/nathan/2026-11-25-agent-security-architecture, and Notes, "Security and access".

## Setting up a machine

- **A server VM for agents.** Login user without sudo, lingering, Tailscale SSH, herdr, the node. Now:
  `docs/drafts/homeserver-guides.md`.
- **Dev servers on a remote machine.** Vite refuses host names it doesn't know (403), and `crypto.randomUUID` exists
  only on HTTPS pages or localhost; use the tailnet's full name and allow it. Now: this conversation (25 November).
- **Syncing notes, skills and instructions between machines.** Now: `docs/drafts/sync-how-to.md`.
- **Account usage and rotation.** Now: `docs/drafts/account-usage-and-rotation.md`.
