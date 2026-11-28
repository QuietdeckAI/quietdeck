> **Superseded (27 Nov 2026):** Claude sign-ins, their usage and the coordinator API moved to Fleetlayer's
> claude-auth-manager plugin. quietdeck-node reads no usage at all now: Fleetlayer's readings have it.

# Account usage and the rotation add-on

Draft by the claude rotation tab, for the quietdeck tab to edit into `docs/`. Commands marked *(run)* were run while
writing it; the rest still have to be run on a fresh machine before the guide counts as done.

Claude and ChatGPT subscriptions have limits: a weekly allowance, sometimes a 5-hour window, and for some models a week
of their own. When you run many agents, you want to see how close each subscription is to its limits before an agent
stops halfway through a task. Quietdeck's node reads that usage on each machine and sends it to the deck, which shows
every subscription on one screen. If you have several Claude subscriptions, the optional rotation add-on moves your
Claude Code sessions between them as they fill up.

## What the node reads

The node's usage module runs as part of the node on each machine. It needs no tokens of its own and nothing in your
secrets manager: it reads the sign-ins that Claude Code and Codex already keep on that machine, and the tokens never
leave the machine. What reaches the deck is the readings: the percentages, the reset times, which plan and whether the
sign-in works.

### Claude

The node watches every Claude Code sign-in on the machine:

- **Your normal Claude Code sign-in**, the one `claude auth login` makes. On macOS Claude Code keeps it in the
  Keychain (the item `Claude Code-credentials`); on Linux in `~/.claude/.credentials.json`.
- **Extra accounts** that the rotation add-on keeps, one folder each under `~/.claude/accounts/`. Your normal sign-in
  is account 1 and has no folder there, so the first extra account's folder is `~/.claude/accounts/2`. Each folder's sign-in is in the Keychain under its own item on macOS,
  and in `<folder>/.credentials.json` on Linux. Without the add-on there's only your normal sign-in.

For each sign-in it reads:

| Reading | Where from |
|---|---|
| The week, the 5-hour window (on plans that have one), and model weeks such as Fable's, each with percent used and reset time | Anthropic's usage endpoint, polled at most every 5 minutes per account. A session that's working reports the readings its responses carry, and an account whose sessions report readings isn't polled at all |
| The email, organisation, plan and plan size | Anthropic's profile endpoint, asked once per sign-in |
| Whether the sign-in works | The sign-in's expiry, and whether Anthropic accepts it |

Claude Code refreshes an expired sign-in itself, but only while a session runs on it. So an account nobody is using
would go stale overnight; the node keeps it fresh by running `claude -p /usage` on it, which reads its usage in
Claude Code and refreshes the sign-in on the way. That uses no model tokens.

If Anthropic's usage endpoint refuses a request (too many requests, or an outage), the node waits 2 minutes before
asking again, then twice as long each time up to 15 minutes, or as long as the answer's `Retry-After` says.

### Codex

The node reads the ChatGPT subscription Codex is signed in to (`codex login`, kept in `~/.codex/auth.json`). Codex has
one sign-in per machine, which shows on the deck as `codex`. A Codex signed in with an API key has no plan limits, so
it doesn't appear.

| Reading | Where from |
|---|---|
| The primary and secondary windows, each with percent used, its length and reset time, and the plan | ChatGPT's usage endpoint (the one Codex's `/status` reads), polled at most every 5 minutes |
| The same windows, between polls | The snapshot Codex writes into its session log after each response (`~/.codex/sessions/<year>/<month>/<day>/rollout-*.jsonl`) |

Codex doesn't name the windows: the length tells you which is which. On some plans the primary window is a week (10,080
minutes) and there's no secondary one.

### Settings

The usage module is on by default. `node.yaml` can change it:

```yaml
usage:
  enabled: true                # read usage on this machine
  poll_minutes: 5              # the longest an idle account goes between polls
  codex: true                  # read the Codex sign-in too
  serve_coordinator_api: true  # serve the rotation add-on's API (below)
```

## The Usage screen

*(The quietdeck tab: describe the screen as built. What the node sends for it:)*

- **One row per subscription.** The same Claude account signed in on two machines is one row: the deck matches them by
  the account's ID (Anthropic's account and organisation IDs; for ChatGPT, the account ID) and shows the newest
  reading, and which machine it came from.
- **Each window's bar**: percent used and when it resets. A window whose reset time has passed counts as empty until
  the next reading.
- **A warning** on a sign-in that needs you: it was rejected, it expired while nothing could refresh it, or it's
  signed in to a different account than before. **Sign in** starts the browser sign-in on the machine it's on. Codex
  signs in from a terminal on that machine with `codex login`.
- **Combined bars** for all your Claude accounts together, weighted by plan size: an account on Anthropic's largest
  plan counts ten times a Pro account, and Max 5x five times. An account that needs a sign-in is left out.
- **Check now** reads every account again, its usage included if the last reading is more than a minute old.

## The rotation add-on (optional)

The add-on is a Claude Code plugin for people with more than one Claude subscription. It keeps a list of accounts on
each machine, and when the account a session is on gets close to its limit (90% of the 5-hour window or 98% of the
week), it moves the session to another account that has room, without restarting it. It reads the usage from the
node: the node serves the add-on's API on `~/.claude/account-rotator/coordinator.sock` (while `serve_coordinator_api`
is on), so the add-on needs nothing else running.

Check your plan's terms before you use it. Claude Code's
[legal page](https://code.claude.com/docs/en/legal-and-compliance) says the advertised limits of Pro and Max assume
ordinary, individual use, and that Anthropic may enforce its rules on sign-ins without notice. The add-on signs
accounts in only through Claude Code's own `claude auth login` and keeps no tokens.

### Install it

In Claude Code, on each machine where you want sessions to rotate:

```text
/plugin install account-rotator --marketplace ndbroadbent/claude-account-rotator
```

Then turn on auto-update, or you stay on the version you installed: in `/plugin`, open **Marketplaces**, pick
**claude-account-rotator** and choose **Enable auto-update**. To update by hand *(run)*:

```bash
claude plugin marketplace update claude-account-rotator
claude plugin update account-rotator@claude-account-rotator
```

Start a new session. The add-on finds the node on its socket, and a band above the prompt asks you to sign in to any
account that needs it.

### Add an account

Click the usage in Claude Code's footer to open the accounts panel, and press **+ Add an account**. A browser opens for the sign-in: sign in to the
other subscription (on claude.ai, sign out first or pick its organisation, or you sign in to the same account again,
which the add-on refuses).

On a machine with no browser, such as a server you reach over SSH, sign in from a terminal there instead. Press
**+ Add an account** to make the account's folder, then run, with its number:

```bash
n=2; CLAUDE_SECURESTORAGE_CONFIG_DIR=~/.claude/accounts/$n CLAUDE_CONFIG_DIR=~/.claude/accounts/$n claude auth login
```

The node picks the new sign-in up within a minute, and the account shows on the Usage screen.

### Where sessions go

The add-on decides where each session goes; the node only reports the accounts. The panel's **Usage based rotation**
setting picks how: **soonest reset** (the account whose week resets first, so no allowance goes unused), **most room**,
**next in list**, or **disabled**, where a session stays on its account until you pick another with **Use now**. A
session in the middle of a turn finishes it before it moves.

### Remove it

Remove each extra account in the panel, which signs it out. Then run `claude plugin marketplace remove
claude-account-rotator`, which uninstalls the plugin, and delete `~/.claude/accounts`. The node goes on reading your
normal sign-in. Leave `serve_coordinator_api` on or turn it off; with no plugin, nothing uses the socket.

## Troubleshooting

- **An account shows "needs a sign-in" but works in Claude Code.** The node couldn't read the sign-in. On macOS, the
  Keychain may be locked (the node logs it once); unlock it, then press **Check now**.
- **Readings stop updating for one account.** Anthropic is refusing its usage requests; the node backs off for up to
  15 minutes. Its log says so: `~/.claude/account-rotator/coordinator.log`.
- **The add-on says it can't reach the coordinator.** The node isn't running, or `serve_coordinator_api` is off.
  Check `quietdeck-node` is running on that machine.
