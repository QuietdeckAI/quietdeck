# Agents propose, you approve

Agents that run for you soon want to change the system they run in. One wants a webhook so a service's alerts reach
it. Another wants a Gmail filter while it fixes a build, a new session to hand work to, a thread watched so a reply
pings your phone, or a rule so it can archive a sender's mail without asking.

In Quietdeck every change like that reaches you as a proposal, with the agent's reason. You approve it with a click,
and nobody edits a config file to make it happen. This guide explains why, shows how each kind of proposal looks today,
and says what stays automatic.

It's for people using Quietdeck and for people building agent systems of their own. [How Quietdeck works](../concepts.md)
explains cards, approvals and rules.

## Why changes to the system go through you

A card about one email does one thing, once. A change to the system keeps working after the task that asked for it:

- **A webhook** is a URL on the public internet that puts a stranger's text in front of an agent.
- **A Gmail filter** hides mail from you.
- **A rule** lets things happen without you.
- **A session** costs money and needs watching.

These are the changes you most need to know about, and the ones an agent is most tempted to make quietly. There are
two usual ways to handle them, and both go wrong. If the agent edits the config file itself, nobody reviews the change.
If it asks you to edit the file, you get a chore: learning the format, finding the right place, and a request that sits
in a chat until you get round to it.

A proposal avoids both. It arrives with the agent's reason, where you already decide everything else, and takes one
click. What you approved is recorded in History, and you can revoke it later from the same screen.

**An agent can't approve its own proposal.** On the deck, approving an endpoint, making a rule or watching a thread
takes the person signed in to the web app with a passkey. Any request that carries a token is refused, so a session's
token can't do it, whatever the agent tries.

## What a proposal looks like today

### A webhook endpoint

The agent that owns a service proposes an endpoint for its alerts:

```sh
quietdeck hooks create --label "Cluster alerts" --kind alertmanager --to "infra@build-server"
```

It shows in Settings → Webhooks with its label, where its events go and which session proposed it. Until you press
**Approve**, the URL answers 404 and receives nothing. **Delete** turns it off for good.
[Send alerts to the agent that owns them](alerts-to-owners.md) walks through it.

### A temporary Gmail filter

A session watching CI asks for a filter that keeps the failure emails out of your inbox while it works:

```sh
quietdeck lease propose gmail-filter --account work \
  --query 'from:notifications@github.com subject:("Run failed")' \
  --reason "I'm fixing the build and watching CI myself" --wait
```

The request is a row on the Threads screen. Approving it approves that query on that account for every session, so
the next session that asks for the same filter gets it at once. Settings → Leases lists the filters you've approved;
revoke one there to need a card again. Every filter lease ends by itself when its session goes idle or closes. The
setting **Require a proposal for every new filter** is on by default. [Leases](../leases.md) has the details.

### A watched thread

A reply on a watched thread may ping your phone, so a watch is yours to start. An agent proposes one with a `watch`
action on a card, and approving the card starts it. You can also watch a thread from a card's menu. Agents can read the
list (`quietdeck watches`), but adding or stopping a watch on the deck takes you.

### A new session

The triage agent proposes a session with a `new_session` action on a card: the machine, the tab, the working directory
and the first prompt. A `resume` action brings back a session that ended. Approving the card starts it.

The triage agent can't do it any other way. It reads strangers' mail, so the deck refuses `quietdeck session start`
and `quietdeck session resume` from any session a runner works for, and says to post a `new_session` or `resume` card
instead.

Other sessions can start or resume one themselves, with `quietdeck session start` or `quietdeck session resume`, the
same as running herdr on that machine: a session starting a helper is ordinary work. The deck records each start in
History with who asked for it ("asked by `<tab>@<machine>`"). Permission flags never come from the deck: each machine
sets them in its own `node.yaml`.

### Rules

You make rules yourself, on the Rules screen. An agent can list them (`quietdeck rules list`) and ask to use one for
a card (`quietdeck card add card.json --auto --grant g_3`), but it can't create or change one. The project's policy in
`deck.yaml` caps what any rule can allow. [Automatic actions](../getting-started.md#automatic-actions-rules-and-policies)
covers both.

When a session ends, its rules are archived. A later card that forwards to a new session in the same tab offers to
move them there, and you decide.

### "Don't ping for this again"

This one works the other way: you tell the agent. Tapping a ping opens it on the Rules screen, where
**Don't ping for this again**, with a note if you like, goes back to the agent that sent it. The agent picks it up
with `quietdeck pings feedback` at the start of its next round and makes it a rule of its own, and the ping shows when
it did. You don't write that rule anywhere.

The same goes for a **Comment** on a History row or a timeline entry. It reaches the agent behind it, and anything it
wants to do about it comes back as a card.

## What stays automatic, and why

Asking you about everything would be as bad as asking about nothing: you'd stop reading and approve by habit. Some
things don't need you, and they share a pattern. They're plain code doing something you already agreed to, or they
undo themselves.

- **The runner carries out what you approved.** It's plain code, with no model in it. It archives, labels and saves
  drafts for cards you approved, and only the action types listed in its `runner.eligible`.
- **Rules you granted.** A card under a rule runs at once, within the project's policy, and still shows in the Inbox's
  timeline of what was done without asking. Automatic resumes and new sessions stop at 3 in any 24 hours.
- **Things that do nothing outside Quietdeck.** A heavy-build slot only makes big builds take turns on a machine, so it
  needs no approval. Leases end on their own.
- **Pings.** An agent pings without asking, because a ping that waited for approval would be too late. Each one
  carries the agent's reason, is recorded, and counts against a weekly budget the Rules screen shows. Nothing is held
  back, and "Don't ping for this again" is always one tap away.
- **The undo window.** Even an approval waits a few seconds (3 by default, up to 60) with an **Undo** button before
  anything runs. [While you read, nothing new lands](while-you-read.md) explains it and the other guards on Approve
  All.

## The dead man's switch is plain code on purpose

Alerts that reach you through Matrix have a weak spot: a quiet room might mean all is well, or that something on the
way died. Quietdeck's answer is a dead man's switch, and it's deliberately not an agent.

Alertmanager's Watchdog alert, which always fires, posts to a Matrix room of its own every 30 minutes. The node's
Matrix reader turns each message there into a GET to a ping URL, such as a Healthchecks.io check, at most once a
minute. Nothing about it reaches the deck or an agent. If Prometheus, Alertmanager, the bridge, the homeserver or the
reader stops, the pings stop, and the checking service emails you.

No card, no agent and no tokens are involved, because this check has to work when the rest is broken. An agent in the
loop would cost tokens every half hour and be one more thing that can fail silently. It's set up once, under
`matrix.heartbeats` in the node's `node.yaml`; [Bring Matrix messages to the triage agent](matrix-source.md#a-dead-mans-switch-for-the-alert-path)
shows how. For monitoring in general (heartbeats for scheduled jobs, health checks that catch a service that's up but
not working), see Fleetlayer's [monitoring guide](https://fleetlayer.ai/guides/monitoring/).

Heartbeats for monitored sessions, and subscriptions that send a channel's messages to the agent in a role, are
planned, not built.

## If you're building something like this

- **Put approval where a token can't reach it.** Quietdeck's approval endpoints accept only the person's signed-in
  browser session and refuse any request that carries a token.
- **Approve a kind, not each instance.** One approval of a filter covers every session that asks for the same query
  on the same account later, so you aren't asked the same question twice.
- **Make every grant end or be revocable.** Leases expire; endpoints, approved filters and rules can be deleted or
  revoked from the screen that lists them.
- **Show what ran without asking.** Automatic actions go on a timeline you see, not only into a log.

## Where this lives in Quietdeck

- Webhook endpoints and their approval:
  [crates/quietdeck-server/src/hooks.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-server/src/hooks.rs)
- Leases, approved filters and the proposal setting:
  [crates/quietdeck-server/src/leases.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-server/src/leases.rs)
- Rules, policies and the daily cap on automatic sessions:
  [crates/quietdeck-server/src/rules.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-server/src/rules.rs)
- Pings, their feedback and thread watches:
  [crates/quietdeck-server/src/pings.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-server/src/pings.rs)
- Starting, resuming and closing sessions:
  [crates/quietdeck-server/src/control.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-server/src/control.rs)
- Which routes only the person can call:
  [crates/quietdeck-server/src/api.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-server/src/api.rs)
  and [crates/quietdeck-server/src/auth.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-server/src/auth.rs)
- The card actions (`watch`, `new_session`, `resume`, `lease`):
  [crates/quietdeck-types/src/action.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-types/src/action.rs)
- The commands agents use:
  [crates/quietdeck-cli/src/main.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-cli/src/main.rs)
- The dead man's switch:
  [crates/quietdeck-node/src/matrix.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-node/src/matrix.rs)
  and `matrix.heartbeats` in
  [deploy/node.example.yaml](https://github.com/QuietdeckAI/quietdeck/blob/main/deploy/node.example.yaml)
