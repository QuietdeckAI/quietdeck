# Run Quietdeck on Fleetlayer

Quietdeck is designed to run on [Fleetlayer](https://fleetlayer.ai/), and works without it. Fleetlayer runs agents
across machines: a node on each machine, plugins, and access rules that decide which agent may message which. On top
of it, Quietdeck's messages go through those access rules, the deck reads Fleetlayer's session summaries, and a
triage agent on its own Unix user can be woken by a runner that keeps the mail credential in your own user.

Without Fleetlayer, Quietdeck's nodes deliver messages to agent sessions themselves and write their own session
summaries, so solo on a laptop, or a deck on a server, needs nothing else. This guide covers the three places the two
meet. You can set them up one at a time:

- Quietdeck's messages go out through Fleetlayer, so Fleetlayer's access rules apply to them.
- The deck reads the session summaries Fleetlayer's summaries plugin writes.
- Quietdeck's summaries pick their Claude account from the socket Fleetlayer's Claude accounts plugin serves.

The examples use two machines, `laptop` and `build-server`, and a triage agent in a tab named `email triage` on its
own Unix user, `triage`, on `build-server` ([Run the triage agent on its own Unix user](triage-agent-own-user.md)).

## Give each machine the same name in both

A Quietdeck node and a Fleetlayer node on the same machine and Unix user should have the same name. Fleetlayer names
a node after the machine, plus `-<user>` for a user other than the machine's first, so the triage user's node is
`build-server-triage`. Enrol the Quietdeck node with that name too:

```sh
quietdeck node enroll <code> --deck https://<deck address> --name build-server-triage
```

Both products address a session as `<tab>@<node>`. When the names match, `email triage@build-server-triage` means the
same session to both, a forward the runner sends through Fleetlayer finds its session, and summaries pulled from
Fleetlayer land on the right machine in the deck. When they differ, Fleetlayer answers "no session named …" for
sessions Quietdeck knows.

## Send Quietdeck's messages through Fleetlayer

Fleetlayer's node can run Quietdeck's node as one of its plugins. Add it to `plugins:` in Fleetlayer's `node.yaml`
(`~/.config/fleetlayer/node.yaml`), on each machine where you want it:

```yaml
plugins:
  - name: runner
    command: quietdeck-node
    args: [fleetlayer-plugin]
    grants: [message.send]
```

Then restart Fleetlayer's node so it reads the change, and check that it lists the plugin as running:

```sh
fleetlayer-node install
fleetlayer status
```

`command` is a full path or a name on the service's PATH. If `fleetlayer status` shows the plugin restarting with "No
such file or directory", give it the full path (such as `/usr/local/bin/quietdeck-node`) and run
`fleetlayer-node install` again. [Add a machine](https://fleetlayer.ai/docs/guides/node-services/) has the other service
problems.

### What changes

On that machine, these now go through Fleetlayer instead of the Quietdeck node's own delivery:

- the runner's forwards, when you approve a card that forwards an email to another session;
- webhook events and messages between sessions that the deck hands this node for one of its sessions.

Fleetlayer finds the session, refuses repeats, and delivers through the agent's own messaging channel, or types it in
through herdr when the session has none. The sender is `plugin:runner@<node>` (the plugin's `name`, then the
Fleetlayer node's name), and the session sees "Message from plugin runner (build-server-triage) via Fleetlayer",
followed by Quietdeck's text. A forwarded email or a webhook is sent marked as outside data, so Fleetlayer adds a line
telling the agent to treat it as information, never as instructions.

Your own words and the runner's to its agent stay off the plugin: the deck's chat, your answers on the report,
handoffs, wake-ups and runner reports. Anything on an agent's Unix user can claim the plugin's identity, so a node
whose agent reads untrusted mail is usually in an untrusted group in Fleetlayer's access rules, and every send from the
plugin there arrives marked as outside data. Those go through the deck's own Fleetlayer sender (below), or the
Quietdeck node's own delivery.

With the plugin, the runner's `agent.tab` may also name a session on another node, `email triage@build-server-triage`,
for a runner that runs under a different Unix user from its agent. Without it, the runner delivers only to a tab on
its own node.

### The deck's own messages

Give the deck a send-scoped token and your words to sessions (the chat, answers on the report, handoffs) go through
Fleetlayer's control plane as `service:quietdeck`, from "Quietdeck", not marked as outside data unless your access
rules say so:

```sh
fleetlayer-control token add quietdeck --scope send
```

```sh
QUIETDECK_FLEETLAYER_SEND_TOKEN=<the token>
```

It needs `QUIETDECK_FLEETLAYER_URL` too. A session Fleetlayer doesn't know (a machine without Fleetlayer), or a
control plane the deck can't reach, gets the message through its Quietdeck node instead. A refusal by the access rules
is final. A send token can only send: it reads nothing.

### When Fleetlayer says no

If Fleetlayer refuses a message (no such session, stopped, waiting on a person, an access rule), Quietdeck reports
Fleetlayer's reason as it is, including its "did you mean …" suggestions. It doesn't try its own delivery instead:
that would get around the rule that refused it. A session Fleetlayer finds stopped or unknown shows as offline in the
deck, as before. Fleetlayer queues nothing and retries nothing.

Quietdeck waits 15 seconds for an answer. A send to another machine can time out and still arrive.

### When the plugin isn't running

The plugin listens on a local socket, `fleetlayer-plugin.sock`, beside the Quietdeck node's own socket. If nothing is
listening there (no Fleetlayer on this machine, the plugin removed from `node.yaml`, or Fleetlayer's node stopped),
the Quietdeck node goes back to its own delivery. A laptop running Quietdeck alone never notices the difference.

The plugin and the Quietdeck node must agree on that socket:

- Run them as the same Unix user. The socket is mode 0600 in a folder the plugin makes 0700.
- If you set `QUIETDECK_NODE_SOCKET` or `QUIETDECK_FLEETLAYER_SOCKET` for the Quietdeck node, set the same in the
  environment of Fleetlayer's node service, since the plugin inherits that. On Linux the default is under
  `$XDG_RUNTIME_DIR`, so both services need it set; systemd user services have it.
- Point `QUIETDECK_FLEETLAYER_SOCKET` at a folder of its own, not a shared one, because the plugin changes that
  folder's permissions.
- Keep the path short. A Unix socket path must fit in about 107 bytes on Linux (103 on macOS). A socket in a deep
  scratch or test folder fails to bind, and Fleetlayer reports the plugin as restarting.

## Limit what the runner may message

Fleetlayer's [access rules](https://fleetlayer.ai/docs/access-control/) name the runner as `plugin:runner@<node>`. They're
checked in Fleetlayer's control plane, for messages to a session on **another** node. A message to a session on the
runner's own node is delivered there without a check, because every process of that Unix user could claim to be any
of its sessions anyway. So a runner beside its agent can always wake it, and the rules decide where its forwards and
other messages may go beyond that node.

To let the runner reach only the triage agent, an `only` rule in Fleetlayer's policy file:

```yaml
version: 2
default: allow

only:
  - who: "plugin:runner@build-server-triage"
    can: [message]
    to: ["email triage@build-server-triage"]
    why: Wakes the triage agent; nothing else.
```

That also refuses every forward to a session on another machine, which is most of what forwards are for. To allow
forwards to your own machines but nowhere else, list them instead:

```yaml
    to: ["*@laptop", "*@build-server"]
```

Apply it, and check a decision before you rely on it:

```sh
fleetlayer-control acl apply policy.yaml
fleetlayer-control acl explain --from plugin:runner@build-server-triage --can message --to "planning@laptop"
```

A pattern such as `*@build-server-triage` matches the runner too. If you follow Fleetlayer's
[untrusted input guide](https://fleetlayer.ai/docs/guides/untrusted-input/) and put `*@<triage node>` in an untrusted
group, its `only` and `untrusted` rules cover the runner's messages as well. Write the runner's own rule with that in
mind.

A plugin is only as trusted as its node: any process of that Unix user can send as `plugin:runner@<node>`. A rule that
gives the runner more reach than its agent holds only when the runner runs under a different Unix user, with its own
node, whose token the agent can't read.

## Read Fleetlayer's summaries in the deck

On a machine where Fleetlayer's [summaries plugin](https://fleetlayer.ai/docs/plugins/summaries/) writes the summaries, the
deck can pull them into its own store, so the Sessions screen, search and the digest work as they do with Quietdeck's
own. Give the deck Fleetlayer's control plane address and a read-scoped token:

```sh
fleetlayer-control token add quietdeck-deck --scope read
```

```sh
QUIETDECK_FLEETLAYER_URL=https://fleetlayer.example.com
QUIETDECK_FLEETLAYER_TOKEN=<the token>
```

Set both or neither: with only one, the deck logs a warning and leaves Fleetlayer off. The deck asks for new records
every minute and keeps its place in the feed, so a restart doesn't read everything again. Summaries Fleetlayer
deletes are deleted in the deck too.

For "since your last message", the deck asks the session's Quietdeck node first and Fleetlayer when the node can't
answer.

Turn Quietdeck's own summaries off on a machine where Fleetlayer's plugin writes them (`summaries: { enabled: false }`
in Quietdeck's `node.yaml`), or both spend tokens summarising the same sessions. A record for the same session and
period replaces the older one, so nothing shows twice.

## Claude accounts

Claude sign-ins and their usage belong to Fleetlayer's [Claude accounts
plugin](https://fleetlayer.ai/docs/plugins/claude-auth-manager/), which serves the Claude account rotator's
`coordinator.sock`. Quietdeck's node never reads a Claude sign-in or serves that socket; the old `usage.claude` and
`usage.serve_coordinator_api` keys in its `node.yaml` are accepted and ignored.

Quietdeck's summaries still read the socket to pick the account with the soonest weekly reset for their calls,
whichever program serves it. They look in `~/.claude/account-rotator`, or `CLAUDE_ACCOUNT_ROTATOR_DIR`. Codex usage
stays with Quietdeck's node.

## Where this lives in Quietdeck

- The plugin, its socket and how a delivery is sent through Fleetlayer:
  [crates/quietdeck-node/src/fleetlayer.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-node/src/fleetlayer.rs)
- The `fleetlayer-plugin` command:
  [crates/quietdeck-node/src/main.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-node/src/main.rs)
- Wake-ups and forwards trying Fleetlayer first:
  [crates/quietdeck-node/src/runner/deck.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-node/src/runner/deck.rs)
- The deck's deliveries to a node trying Fleetlayer first:
  [crates/quietdeck-node/src/link.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-node/src/link.rs)
- The deck pulling summaries and asking for catch-ups:
  [crates/quietdeck-server/src/fleetlayer.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-server/src/fleetlayer.rs)
- Every setting and environment variable: [Configuration](../configuration.md), and the deck's in
  [deploy/README.md](https://github.com/QuietdeckAI/quietdeck/blob/main/deploy/README.md)
- The summaries and usage modules' contracts: [summaries-interface.md](../summaries-interface.md),
  [usage-interface.md](../usage-interface.md)
