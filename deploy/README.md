# Deploying Quietdeck

The contract between Quietdeck and whoever hosts it. Change this file in the same commit as any change to ports,
paths or environment variables.

Quietdeck has two parts to deploy:
- **The deck:** one container, one replica, a persistent volume. It holds no service credentials.
- **A node on each machine that runs agent sessions:** a user service next to herdr and Claude Code.

## The deck

### Image

- Built from `deploy/Dockerfile` (multi-stage: the Rust binary, then a distroless `nonroot` image holding that one
  binary, with the web app built in a Node stage and embedded). Build from the repository root:
  `docker build -f deploy/Dockerfile -t quietdeck .`
- Binary `quietdeck-server` (in `/usr/local/bin`, so `docker exec <container> quietdeck-server …` finds it), entrypoint
  `["/usr/local/bin/quietdeck-server", "serve"]`. Runs as uid and gid 65532.
- Push it to your own registry as `<registry>/quietdeck/deck:<git short SHA>`. No public image is published yet.
- Root filesystem can be read-only. The deck writes only to the data volume and `/tmp` (an `emptyDir` is enough).

### Ports

| Port | Serves | Exposure |
|---|---|---|
| 8080 | The web app, the REST API (`/api/*`), server-sent events, the node WebSocket (`/api/nodes/ws`), terminal WebSockets, and MCP at `/mcp` | **Private network only**, through an HTTPS ingress. WebSocket upgrades and server-sent events must pass through unbuffered. Never behind Cloudflare or any other public proxy |
| 8081 | Incoming webhooks: `POST /hooks/<endpoint id>/<secret>`. Nothing else | Public, through a Cloudflare Tunnel whose ingress sends only `^/hooks/` here and answers everything else with `http_status:404` |

Health: `GET /healthz` on both ports answers `200 ok` once the database is open and migrated, and 503 before that. Use
it for liveness and readiness probes.

### Data volume

Mounted at `/data`, ReadWriteOnce, owned by gid 65532 (`fsGroup: 65532`).

| Path | What |
|---|---|
| `/data/quietdeck.db` | SQLite, in WAL mode (`journal_mode=WAL`, `synchronous=NORMAL`), with `quietdeck.db-wal` and `quietdeck.db-shm` beside it. Migrations run at start-up |
| `/data/deck.yaml` | The deck's config: accounts, boards, projects, ping budget, lease fallback. Read at start-up; an example is in `deploy/deck.example.yaml`. Missing means defaults |

**Backups** need no deck code. Take an online copy of the live database with `sqlite3 /data/quietdeck.db ".backup
'<target>'"` or `VACUUM INTO '<target>'`, never a file copy, and copy `deck.yaml` alongside. Because the database is in
WAL mode, the backup process needs **write** access to `/data` (SQLite readers write to the `-shm` file): mount the
volume read-write and run as gid 65532. On a ReadWriteOnce volume, the backup pod must land on the deck's node.

**One replica**, rolled with `RollingUpdate` (`maxSurge: 1`, `maxUnavailable: 0`) and a readiness probe on
`/healthz`: the new deck starts on the same node (the ReadWriteOnce volume keeps it there), takes traffic once it's
ready, and the old one then closes its node links and event streams and stops. The two share the SQLite file for a
few seconds, which SQLite's locking allows, so every migration only adds (CI refuses one that drops or renames,
below). Never run two replicas.

### Environment

| Variable | Required | Default | What |
|---|---|---|---|
| `QUIETDECK_PUBLIC_URL` | yes | | The app's one stable HTTPS origin, e.g. `https://quietdeck.example.com`: a host name, not an IP address. It's the passkey relying party (its host is the RP ID), the only origin whose pages may act with the sign-in cookie, the origin push links open, and the URL nodes connect to. Changing it means registering passkeys and installing the web app again. `http://localhost:<port>` works for a deck on your own machine |
| `QUIETDECK_HOOKS_PUBLIC_URL` | yes, for webhooks | | The public origin of the hooks port through the tunnel, e.g. `https://hooks.example.com`. Used to print endpoint URLs |
| `QUIETDECK_VAPID_PUBLIC_KEY` | yes, for push | | Web Push VAPID public key, base64url (uncompressed P-256 point) |
| `QUIETDECK_VAPID_PRIVATE_KEY` | yes, for push | | The matching private key, base64url. A secret. Generate a pair with `quietdeck-server vapid-keys` |
| `QUIETDECK_VAPID_SUBJECT` | no | `QUIETDECK_PUBLIC_URL` | A contact for push services: `mailto:you@example.com` or an `https://` URL. Apple rejects localhost and `.local` addresses |
| `QUIETDECK_DATA_DIR` | no | `/data` | |
| `QUIETDECK_CONFIG` | no | `$QUIETDECK_DATA_DIR/deck.yaml` | |
| `QUIETDECK_LISTEN` | no | `0.0.0.0:8080` | |
| `QUIETDECK_HOOKS_LISTEN` | no | `0.0.0.0:8081` | |
| `RUST_LOG` | no | `info` | Logs never contain message bodies, webhook payloads or card contents |
| `QUIETDECK_DEV_TRUST_LOCAL` | no | off | Development only: every browser request counts as the signed-in owner. Never set it on a deck anyone else can reach |

Without the VAPID variables the deck runs and says in Settings that push is off. Changing the VAPID key pair drops
every device's push subscription.

### Sign-in

Passkeys only. The first passkey is registered with a one-time setup code (`quietdeck-server setup-code`, run in the
container, valid 15 minutes, once); later ones are added in Settings while signed in. A browser session is an
`HttpOnly`, `SameSite=Strict` cookie (`Secure` when the public URL is HTTPS) that ends after 30 days without use, and
the deck refuses any state-changing request with it whose `Origin` isn't `QUIETDECK_PUBLIC_URL`. Nodes and the CLI
authenticate with their own tokens, never the cookie.

### Ingress notes

- Serve 8080 at `QUIETDECK_PUBLIC_URL` through an HTTPS ingress on the private network (a cluster ingress with a
  private address and your certificate, or `tailscale serve`). It must support WebSockets and must not buffer
  server-sent events (`/api/events`).
- **Keep 8080 off every public network.** Sign-in protects every endpoint, but the deck is a single-user control
  plane: there's no reason for the internet to reach it. A NetworkPolicy that admits 8080 only from the ingress's pods
  is a good second fence. With Docker, publish 8080 on `127.0.0.1` and run the HTTPS proxy on the same host.
- Nodes connect to `QUIETDECK_PUBLIC_URL` too. They authenticate with their node token, so the ingress must not demand
  any other sign-in in front of the deck.
- The deck makes outbound requests only to Web Push services (`*.push.apple.com`, `fcm.googleapis.com`,
  `updates.push.services.mozilla.com`).

### Resources

A request of 50m CPU and 64 MiB, and a limit of 512 MiB, is plenty for one user.

### Continuous deployment

A push to `main` deploys the deck once CI passes, with no hand-off (`.github/workflows/ci.yml`, modelled on Talira's):

1. The `image` job builds `deploy/Dockerfile` with the commit and build time baked in (`QUIETDECK_COMMIT`,
   `QUIETDECK_BUILT_AT`: Settings' Release panel), checks the image reports its commit, and pushes
   `ghcr.io/quietdeckai/quietdeck/deck:<short SHA>`.
2. The `deploy` job, once `rust`, `web` and `image` pass, commits two lines of the host's values file (for Nathan's
   cluster, `k3s/apps/quietdeck/values.yaml` in `ndbroadbent/homeserver-terraform-ansible`), with a write deploy key
   (the Actions secret `INFRA_DEPLOY_KEY`; it signs the commit too):

   ```yaml
           image:
             repository: ghcr.io/quietdeckai/quietdeck/deck
             # quietdeck commit: <full SHA>
             tag: '<short SHA>@sha256:<digest>'
   ```

   It only moves production forward: a commit that doesn't come after the deployed one (an older run finishing late,
   a re-run) is skipped, checked again before each push, and runs on `main` queue rather than overlap. It refuses a
   migration that drops or renames (the old deck still uses the database during the roll) unless a commit message
   since the deployed one says `[allow-destructive-migration]`.
3. Argo CD syncs it: a PreSync Job backs up the database (an online `VACUUM INTO`, as in "Data volume"), the
   Deployment rolls, and a PostSync Job checks `/healthz`. A failed Job fails the sync and alerts.

Roll back by reverting the deploy commit in the host's repository. Without `INFRA_DEPLOY_KEY` the job only warns, and
deploys are by hand: `scripts/build-images --wait <commit>` builds the images on docspring-dev for the host to pin.
The node binaries (`node-bin`) still come from `scripts/build-images`.

## A node

One per machine that runs sessions, as the same user who runs herdr and Claude Code, so it can reach their sockets and
`~/.claude`.

| Item | Path |
|---|---|
| Binary | `quietdeck-node` (and the `quietdeck` CLI) on `PATH`, e.g. `~/.local/bin` |
| Config | `~/.config/quietdeck/node.yaml` (example: `deploy/node.example.yaml`) |
| Node token | `~/.config/quietdeck/node-token`, mode 0600, written by `quietdeck node enroll <code>` with a one-time code from the deck's Settings |
| Runner's Executor token (only where the runner is on) | `~/.config/quietdeck/executor-token`, mode 0600, or wherever `runner.executor.token_file` points |
| State (offsets, the runner's history IDs, summaries state) | `~/.local/state/quietdeck/` |
| Local socket (CLI, MCP headers helper, menu bar app) | `$XDG_RUNTIME_DIR/quietdeck/node.sock` on Linux, `~/Library/Application Support/quietdeck/node.sock` on macOS, mode 0600 |

**Linux:** a systemd user unit, `deploy/quietdeck-node.service`, installed to `~/.config/systemd/user/`. Enable lingering
(`loginctl enable-linger <user>`) so it runs without a login session. Its `PATH` must include `herdr`, `claude` and,
for the PR detector, `gh`.

**macOS:** a launchd agent, `deploy/com.quietdeck.node.plist`, installed by `quietdeck-node install`.

**Claude Code's inbound setting.** The node delivers messages to Claude Code sessions the way `SendMessage` does
(`docs/claude-code-messaging.md`). Sessions in bypass-permissions mode hold those messages for approval unless
`~/.claude/settings.json` on that machine has `"crossSessionInbound": "accept"`. Set it on every machine whose sessions
run with `--dangerously-skip-permissions` and should receive wake-ups, forwards, relayed messages and webhooks.

The node opens no ports. It needs outbound HTTPS and WebSocket access to `QUIETDECK_PUBLIC_URL` and, with the runner
on, to the user's Executor.
