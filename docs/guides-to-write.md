# Guides to write

Nathan (25 November 2026): "we're implementing so much stuff and solving so many problems in quick sessions, and almost
all of it is needed by everyone to build a robust and secure cluster of AI agents." Each entry below is something we
solved or decided for ourselves that belongs in Quietdeck's published guides. **Add an entry in the same change that
solves the problem**, with where the knowledge lives now, so nothing is lost before the guides are written. Strike an
entry through only when its guide is in `docs/`.

Drafts other sessions wrote for these are in `docs/drafts/`.

Guides about running agents in general belong on Fleetlayer's site (fleetlayer.ai/guides/), agreed with the
`fleetlayer docs` session on 27 November 2026; a struck entry says where its guide is. Quietdeck's guides link there.

## Running agents

- ~~**Moving a session to another machine.**~~ Written: `docs/guides/move-a-session.md`. Handoffs, the access manifest, `handoff claim`, closing the old session.
  Now: `skills/move-session/SKILL.md`, and the quietdeck move (h_2, 24 November).
- **Agents addressed by role, not tab name.** Why a role setting beats a tab name hard-coded across runner configs,
  lease fallbacks, instructions and skills; lightweight agents any session can take on. Now: build plan "Agents are
  addressed by role, not tab name (25 November)". Not built yet.
- **Monitoring the sessions that must stay up.** Long-running agents with responsibilities (triage, infrastructure,
  a queue worker) are marked monitored, and the node pings a heartbeat URL while each runs, so Gatus, Healthchecks or
  Cronitor alerts when one stops; the rest can be closed any time. Agents propose it on a card and the person
  approves. Now: build plan (Nathan, 25 November). Not built yet.
- ~~**No trust prompts for agent sessions.**~~ On Fleetlayer: https://fleetlayer.ai/guides/unattended-agents/. Claude Code keeps folder trust in `~/.claude.json`
  (`projects.<path>.hasTrustDialogAccepted`). Despite what we first thought, a trusted parent doesn't cover the folders
  inside it: Claude Code 2.1.332 prompted in `~/code/obsidian-livesync` on a Mac whose `/` and `~/code` were both
  trusted (27 November), so mark the exact folder every time. It never remembers the home folder, so sessions
  shouldn't start there: the node marks a new session's own folder trusted and moves a home-folder start to
  `~/Notes/Agents/<tab>`. Now: `crates/quietdeck-node/src/control.rs` (25 and 27 November).
- ~~**Every agent-facing command needs a timeout.**~~ On Fleetlayer: https://fleetlayer.ai/guides/unattended-agents/. A CLI waiting on a socket or HTTP call with no limit hangs an
  agent's turn forever when the other end restarts mid-request (a node restart during `msg send`, 25 November): the
  agent can't recover by itself. Quietdeck's CLI: 10 seconds for the local node, 3 to connect and 30 in all for the
  deck, 5 end to end for a message (the deck's delivery 4, a herdr prompt 3). Now: `crates/quietdeck-cli/src/client.rs`.
- ~~**Agent tools must work without a login session.**~~ On Fleetlayer: https://fleetlayer.ai/guides/node-services/. `sudo -u <user>` and cron jobs have no `XDG_RUNTIME_DIR`, so a
  CLI that finds its daemon's socket through it fails for exactly the separate agent users (triage, talira). The CLI
  now falls back to `/run/user/<uid>/`. Now: `crates/quietdeck-cli/src/client.rs` (25 November).
- ~~**Sessions survive a reboot.**~~ On Fleetlayer: https://fleetlayer.ai/guides/node-services/. herdr resumes Claude Code sessions after a VM restart, with their conversation;
  background processes and the session's temporary scratchpad don't survive. Now: the agents VM restart, 25 November.
- **Per-machine instructions.** A shared global instructions file for every machine, plus one file per machine
  (`CLAUDE.this-machine.md`, which the sync leaves out) for what holds only there: no browser, no Touch ID, how to
  reach the other machines. Now: Nathan's global instructions and the homeserver repo's agents VM role.
- ~~**Agents propose, the person approves.**~~ Written: `docs/guides/propose-approve.md`. Subscriptions, monitoring, rules and new sessions are proposed by agents
  on cards and approved by the person, never written into a config file the person has to edit. Now: Nathan, 25
  November (build plan).
- **Two scratchpads per agent.** A wiped one (`/tmp`, the session scratchpad) and a kept, synced one in the notes
  vault, named after the role. Now: Nathan's global instructions, "Agents' scratchpads: one wiped, one kept".
- ~~**Builds off the laptop.**~~ On Fleetlayer: https://fleetlayer.ai/guides/agents-building-software/. Heavy Rust builds and tests on a build server (`scripts/on-dev`), heavy-build leases, why
  a laptop's disk fills. Now: `AGENTS.md`, `docs/leases.md`.

- **Continuous deployment without races.** CI builds the image and commits its digest to the infra repo with a
  deploy key; production only moves forward (an ancestry check before each push), main's runs queue, migrations may
  only add during a rolling deploy, Argo CD backs up before and checks after. Why not an image updater (build order
  isn't commit order). Now: `deploy/README.md` "Continuous deployment", Talira's `internal-docs/deploying.md`.

- ~~**Agents driving your own browser.**~~ On Fleetlayer: https://fleetlayer.ai/guides/browser/. One of the things people do most (Nathan, 25 November): any agent on any
  machine controls the person's real, signed-in Chrome, reusing its cookies, and asks them to sign in in a tab when
  needed. Playwright MCP's extension, its server on the Mac per session (tab groups keep sessions apart), reached over
  SSH or Tailscale SSH; later Quietdeck as the gate (allowlist, approvals, audit, kill switch), never as the pipe. Now:
  build plan "Your own browser, from any agent", `~/Notes/Agents/browser/chrome-control-research.md`.

- ~~**Agents messaging each other: never paste into a terminal.**~~ Written: `docs/guides/agents-messaging.md`. An agent whose `SendMessage` failed (the target was on
  another machine, with Remote Control off) found the tab with `herdr machine list` and pasted its report in with
  `herdr agent prompt`. It landed in the middle of a message Nathan was typing and mangled it. Fix: agents use
  `SendMessage` on the same machine and `quietdeck msg send` across machines or when `SendMessage` says a session
  isn't reachable, and never paste into another Claude session. The relay's own herdr fallback stays (Nathan, 26 Nov
  2026): it's only for sessions with no messaging socket. Now: Nathan's shared CLAUDE.md, "Messaging Nathan's herdr
  sessions" (26 November).

- ~~**Mirrored transcripts get summarised twice.**~~ On Fleetlayer: https://fleetlayer.ai/guides/day-to-day/. On a machine with one Unix user per agent, a root timer
  (`agents-mirror`, every 2 minutes) copies each agent user's Claude project into the main user's home, so its
  memory syncs and its transcripts reach the archive. Anything that sweeps the main user's `~/.claude/projects` then
  sees the other user's live transcript as its own, and summarised it under the wrong node, beside that user's own
  node. Fix: a summariser takes only sessions whose recorded cwd is inside its own user's home. Now: Fleetlayer
  6cc8de8 (plugin) and Quietdeck b306e6a (`summaries/transcript.rs`, `under_home`), 26 November.

## The triage agent

- ~~**Giving an agent with no mail tools the mail itself.**~~ Written: `docs/guides/headless-agent-one-cli.md`. Quietdeck solo's agent can only run quietdeck commands, so
  the runner reads each new message (gog's `--sanitize-content`: no HTML, no links) and puts it in the wake-up between
  `<email>` tags, each message cut at 3,000 characters, at most 25 per wake-up, headers flattened to one line with
  `<` and `>` replaced, and any `</email` in the text broken so it can't close its tag. The untrusted-input line goes
  above them. On the very first run, the inbox's 25 newest messages count as new, so the first round has something to
  triage. Now: `mail_block` in `crates/quietdeck-node/src/runner/mod.rs`, `recent` in `mail.rs` (27 November).
- **rust-embed serves nothing from a debug build on another machine.** The deck embeds the web app with rust-embed,
  which embeds only in release builds: a debug build reads `web/dist` from the build machine's disk at runtime, so a
  debug binary copied elsewhere shows "This build has no web app". Copy release builds. Found while setting up the
  solo toast QA (27 November).
- ~~**A local web app is not safe just because it's on localhost.**~~ Written: `docs/guides/localhost-web-app.md`. Quietdeck solo's deck has no passkeys, and any
  page the person visits can post to `http://127.0.0.1:<port>` (CSRF: form posts and no-cors fetches carry no auth
  but still act) or read it through DNS rebinding. So the deck checks every request's Host against its own loopback
  address and port, signs a browser in only through a one-time link (`quietdeck-solo open`: 5 minutes, used up on
  first visit) rather than trusting by address, needs its own Origin (or Referer) on anything that changes state, and
  answers no CORS preflight, which is what keeps a page from sending a Bearer token cross-origin. Each refusal has a
  test. Footgun: passkeys (webauthn-rs) refuse an IP address as the relying party, so the deck's public URL is
  `http://localhost:<port>` while links use 127.0.0.1. Now: `crates/quietdeck-server/src/solo.rs` (27 November).
- ~~**"Text file busy" in tests that write a script and run it.**~~ Written: `docs/guides/mail-through-gog.md`. A test that writes a fake CLI and executes it at
  once fails now and then under a parallel run: another thread's fork can hold the new file open for writing until
  its exec, and the kernel refuses to run a file open for writing (ETXTBSY). The runner retries a spawn briefly on
  that error, which also covers a CLI being upgraded in place. Now: `spawn` in
  `crates/quietdeck-node/src/runner/gog.rs` (27 November).
- ~~**A headless agent limited to one CLI.**~~ Written: `docs/guides/headless-agent-one-cli.md`. For Quietdeck solo, each message is one `claude -p` turn on the same
  conversation (`--resume=<id>`, the ID from `--output-format=json`'s result, kept in the state directory), the prompt
  on stdin. `--permission-mode=dontAsk` refuses whatever isn't allowed without a prompt nobody would answer,
  `--tools=Bash` with one `--allowedTools=Bash(quietdeck <subcommand>:*)` per triage subcommand leaves only those
  (not `msg send` or `session start`: the agent reads untrusted mail), and `--strict-mcp-config`
  drops the user's MCP servers, so the agent has no mail tools of its own. Footgun: `--allowedTools` takes a list,
  so write it `--allowedTools=<rule>`, or a bare value after it swallows the next argument. Checked for real on 27
  November with Claude Code 2.1.332: `quietdeck --version` ran, `ls /` was refused and listed in
  `permission_denials`, a resumed turn remembered the last, and with the narrowed rules `quietdeck cards` ran while
  `quietdeck msg` and `quietdeck session` were refused. Identity: a turn has no tab, so the node records each turn's
  pid while it runs (removed when the child exits, times out or is killed, so a reused pid inherits nothing) and gives
  the CLI calls it starts the agent's tab and the turn's session ID; a new conversation gets its ID up front
  (`--session-id`). Now: `crates/quietdeck-node/src/runner/headless.rs` and `local.rs` (`HeadlessTurns`).
- ~~**Mail through gog, without Executor.**~~ Written: `docs/guides/mail-through-gog.md`. For Quietdeck solo, `runner.mail: gog` runs the user's own gog CLI
  (signed in with `gog auth add <address> --services gmail`) for every mail tool. Footguns: gog's `gmail history`
  asks Gmail for new messages only and prints bare IDs (no labels, threads, archives or deletions), and gog has no
  getProfile. So the runner reads each new message's labels with `gog gmail raw <id> --format minimal`, starts from
  the newest message's historyId (never a backfill), and finds threads that left the inbox by comparing two complete
  `gog gmail search --ids-only --all in:inbox` listings: a failed or partial listing changes nothing. gog's exit
  codes are stable (5 not found, 4 sign-in needed), so errors are told apart by code, not by text. Every ID goes in
  only if it looks like a Gmail ID and every value as `--flag=value`, so mail content can't become a flag, and a
  draft's body goes on stdin. Now: `crates/quietdeck-node/src/runner/gog.rs` (27 November). Not yet run against a
  real mailbox.
- ~~**The triage agent on its own Unix user and node.**~~ Written: `docs/guides/triage-agent-own-user.md`. It reads untrusted mail with permissions skipped, so no
  credentials or repos may share its user; its own node and herdr; how other sessions reach it. Now:
  `docs/drafts/homeserver-guides.md`, the homeserver repo's agents VM role.
- ~~**While you read the Inbox, nothing new lands.**~~ Written: `docs/guides/while-you-read.md`. Presence, held wake-ups, "while I'm reading" rules (RE2), toasts,
  Approve All's ten-second settle window. Now: `docs/getting-started.md` (a paragraph), commit 3b8de63. Needs a full
  guide with the reasoning: a page that changes under the person is how a wrong approval happens.
- ~~**Archiving in Gmail yourself keeps the report in step.**~~ Written: `docs/guides/archive-in-gmail.md`. Mail archived or deleted on a phone or in the browser
  leaves the inbox, the runner confirms each thread has really gone (every message, not one), the deck resolves the
  cards whose threads have all left, and the agent tidies the report, or marks it done with `quietdeck report done` so the
  Inbox says "All done." The history tool needs `labelRemoved` and `messageDeleted`. Now: `deploy/node.example.yaml`,
  `crates/quietdeck-server/src/mail.rs` (`threads_left`), the email-triage skill (25 November).
- **A smaller context window for the triage agent.** Triage doesn't need a huge context to do its job well, so
  Nathan set the `quietdeck triage` session's autocompact window to 300k tokens (25 November): it compacts sooner and
  each turn costs less. Set it when setting up a triage agent for someone. Now: the triage session's own settings
  (Nathan set it by hand; record the exact setting here when the setup is scripted).
- ~~**What plain code may read.**~~ Written: `docs/guides/headless-agent-one-cli.md`. The runner reads headers, labels and snippets but never logs them; plain code is the
  safe place to read untrusted text (no prompt injection), so "never read content" isn't a rule worth keeping.
  Now: this conversation (25 November), `crates/quietdeck-node/src/runner/mail.rs`.

## Inputs beyond email

- ~~**Matrix as a message source.**~~ Written: `docs/guides/matrix-source.md`. A service user, an end-to-end encrypted room, the device and crypto store kept on
  one node, `quietdeck messages`, `quietdeck-node matrix-check`, cards with account `matrix`. Now: sc-7779,
  `deploy/node.example.yaml`, commits ec81b71 to 23de605.
- **Alerts forwarded to the agent that owns the service.** Subscriptions from a channel to a role, written by agents
  and approved by the person; retries and a fallback to triage. Now: the design sent to planning (25 November). Not
  built yet.

- ~~**Alerts go straight to the agent that owns them.**~~ Written: `docs/guides/alerts-to-owners.md`. A webhook per sender (Alertmanager, Argo CD notifications,
  Gatus), approved by the person, delivered to the owning session as outside data; the triage agent only folds a count
  into the report ("42 alerts about memory; talira infra handled them"), never a card per alert (Nathan, 25 November).
  An alert whose owner is offline falls back to triage. Now: `deploy/README.md` "Webhooks",
  `crates/quietdeck-server/src/hooks.rs`, build plan 3.7.

## Watching everything in the background

- **Every background thing has a check that alerts, and an agent listens.** Heartbeats for scheduled jobs, health
  checks for daemons, checks that catch "running but not working", alerts to a Matrix room, and the owning agent
  fixing it. Now: Nathan's global instructions, "Everything in the background is watched, and an agent fixes it".
- ~~**A dead man's switch for the alert path.**~~ Written: `docs/guides/propose-approve.md`. Alertmanager's Watchdog posts to its own Matrix room every 30 minutes;
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

- **The node's sessions from Fleetlayer's registry.** Where a machine runs Fleetlayer, Quietdeck's node lists its
  sessions from Fleetlayer's `node/sessions` (the same session IDs, tabs and addresses herdr reports, checked side by
  side on the agents VM) and still lets herdr open, close and type into them, so no delivery changes path. Sessions
  still starting (`@<pane>`, `named: false`) and ended ones are left out. Without Fleetlayer, herdr lists them as
  before. Now: `crates/quietdeck-node/src/orchestrator/fleetlayer.rs` (ae06f2f).
- **The Sessions screen from Fleetlayer (in review).** The deck reads `GET /v1/fleet` with its read token, which shows
  every machine, including ones with no Quietdeck node. Fleetlayer keeps ended sessions for only an hour, so the deck
  merges its own rows for history and for machines without Fleetlayer, and hides a running row Fleetlayer no longer
  lists. Now: the cutover's local branch `sessions-fleet`, waiting for Nathan's review.
- ~~**Quietdeck's messages through Fleetlayer.**~~ Written: `docs/guides/with-fleetlayer.md`. On a machine that runs Fleetlayer, add Quietdeck's node to Fleetlayer's
  node.yaml as a plugin (`name: runner`, `command: quietdeck-node`, `args: [fleetlayer-plugin]`,
  `grants: [message.send]`). The runner's forwards and its wake-ups then go through Fleetlayer as
  `plugin:runner@<node>`, the sender access rules name, with an email forward marked as outside data (`untrusted`).
  Without it, or with no Fleetlayer at all (Quietdeck solo on a laptop), the node keeps its own delivery.
  `quietdeck msg send` goes through Fleetlayer's node socket whenever one answers, exactly as `fleetlayer msg send`
  would (Fleetlayer's access rules apply, and replies say `fleetlayer msg send`), and through the deck otherwise. Footgun: a
  Unix socket path must be under about 107 bytes, so a test node's sockets can't live in a deep scratch folder. Now:
  `crates/quietdeck-node/src/fleetlayer.rs` (27 November, the cutover), smoke-tested against a real fleetlayer-node.
- **A server VM for agents.** Login user without sudo, lingering, Tailscale SSH, herdr, the node. Now:
  `docs/drafts/homeserver-guides.md`.
- **Dev servers on a remote machine.** Vite refuses host names it doesn't know (403), and `crypto.randomUUID` exists
  only on HTTPS pages or localhost; use the tailnet's full name and allow it. Now: this conversation (25 November).
- ~~**Reinstalling a launchd agent races.**~~ On Fleetlayer: https://fleetlayer.ai/guides/node-services/. `launchctl bootout` returns before the old job is gone; a `bootstrap`
  straight after fails with "5: Input/output error" and leaves the service down. Wait until `launchctl print
  gui/<uid>/<label>` fails, then bootstrap, retrying a few times. Now: `crates/quietdeck-node/src/service.rs`
  (26 November), and Fleetlayer's `fleetlayer-node/src/service.rs`.
- **Many agents on one GitHub account share one API limit.** `gh run watch` polls every 3 seconds by default, so
  five sessions watching CI used up the account's 5,000 calls an hour, and every session's gh calls failed (HTTP 403)
  for about 35 minutes. Pass `--interval 60`, and check `gh api rate_limit` when gh starts refusing. Now: this entry,
  and the quietdeck tab's memory (27 November).
- **"No route to host" right after installing a new binary on macOS.** A launchd service started from a new path logs
  `No route to host (os error 65)` for its first few seconds, then connects. That's macOS settling its local network
  permission for that binary, not a network fault. The node retries, so nothing needs doing. Seen on the Mac's
  Quietdeck and Fleetlayer nodes (27 November). Now: this entry.
- **Syncing notes, skills and instructions between machines.** Now: `docs/drafts/sync-how-to.md`.
- **Account usage and rotation.** Now: `docs/drafts/account-usage-and-rotation.md`.
- **Building your own frontend on the published API.** The deck's API is published as an OpenAPI 3.1 spec,
  `api/openapi.json` (Apache-2.0, so a frontend can use any licence), served live at `/api/v1/openapi.json`. Every
  route is under `/api/v1`; the old `/api/...` paths answer the same routes for nodes and CLIs already in the field.
  Changes within v1 are additive only (`api/README.md` has the policy). Operations tagged `report` or `presence`
  (`x-quietdeck-ui`) belong to the default web app and can be ignored. How it's kept honest: aide builds the spec from
  the handlers' own types, a test rewrites the file, CI fails if it differs or fails `redocly lint`, and tests fail
  when a route has no summary or isn't in the spec. Footguns: aide documents only routes added with `api_route`
  (plain `route` is invisible, which the test catches); a bare `Path<i64>` gives no path parameter in the spec, so
  paths use named structs (`Path<Id>`); and serde_json's `preserve_order` feature changes key order, so the spec is
  written with every object's keys sorted. Now: `crates/quietdeck-server/src/openapi.rs`, `api/README.md`
  (27 November, the quietdeck api tab). Still open: the API reference page on quietdeck.ai, and Fleetlayer sign-in
  for frontends served from elsewhere.
- **A typed client from the spec, and the two traps it showed.** `clients/typescript` (Apache-2.0) is generated from
  `api/openapi.json` by openapi-typescript over openapi-fetch; CI regenerates it and fails on a diff. Trap 1:
  openapi-typescript 7 makes every field with a `default` required (`defaultNonNullable`), right for answers but
  wrong for requests, so a client had to send every optional field: generate with `--default-non-nullable false`.
  Trap 2: axum answers requests it can't read (bad JSON, wrong content type, bad query) in plain text, so a client's
  error type was `ApiError | string` and `if (error)` didn't narrow; the deck now turns those into `ApiError` JSON
  (`json_errors` in `crates/quietdeck-server/src/openapi.rs`). Both found by building a small third-party frontend
  on the client (`examples/inbox`), which is the test worth repeating for any published API. Now: those files and
  `~/Notes/Agents/quietdeck api/example-frontend-findings.md` (27 November, the quietdeck api tab).
- **Tests that read "now" in a real time zone fail around midnight.** Two summaries tests put activity "30 minutes
  ago" through the engine with the zone set to Pacific/Auckland, and expected today's hour; from 00:00 to 00:30 NZ
  time that activity was yesterday's, so the tests (and main's CI, whose deploy job then skipped) failed every night.
  Fixed by running them in a fixed zone where it's always about noon (`noon_now()` in
  `crates/quietdeck-node/src/summaries/tests.rs`, ccfd523, 28 November, the quietdeck work tab). Anyone writing a
  time test: freeze the clock or pick the zone from the current time; never a real zone with a small offset from now.
- **`gh run list --branch main` lists the Site workflow too.** A watcher that takes the newest run can end up watching
  the site build instead of CI and report success while CI is still running. Filter by `workflowName == "CI"`
  (28 November, the quietdeck work tab). Now: this entry.
- **Documents, Work and the overseer.** How the Work tab is set up: a `work` document written by an overseer agent
  (`quietdeck-server documents-add --slug work --name Work --tab Work --kind work --writer overseer@<node>` in the deck's
  container, or Settings > Documents), the runner's `agent.tabs` listing the overseer, `session` in its eligible list,
  Quietdeck's Fleetlayer plugin with `sessions.watch` and `session.answer` on each node whose prompts should show, and
  the deck's `/healthz/overseer` in the uptime checker. Now: the overseer skill (`~/.claude/skills/overseer/SKILL.md`),
  `crates/quietdeck-server/src/overseer.rs`, `crates/quietdeck-types/src/document.rs` (28 November, the quietdeck work
  tab). The quietdeck docs tab is writing docs/work.md and docs/guides/overseer.md.
