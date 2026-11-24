# Leases

A lease is a session's claim on something for a while. Quietdeck has two kinds:

- **gmail-filter**: a temporary Gmail filter that keeps mail out of your inbox while a session works on it. A session
  watching CI asks for one so that every "Run failed" email doesn't land in your inbox while it fixes the build.
- **heavy-build**: a slot for a big compile or test run on a machine. A machine has a few slots (2 by default), and a
  session that finds them all taken waits its turn.

Every lease ends on its own: when its session releases it, when its time limit passes (24 hours by default), when its
session has been idle or offline for its idle minutes (30 by default), or when its session closes. The deck checks
every 15 seconds.

## Gmail filters

A session asks with `quietdeck lease propose`:

```sh
quietdeck lease propose gmail-filter --account work \
  --query 'from:notifications@github.com subject:("Run failed")' \
  --reason "I'm fixing the build and watching CI myself" --wait
```

The query needs a `from:` with a full address or domain (`from:notifications@github.com` or `from:github.com`), so a
filter can't catch all your mail. A session holds at most five filter leases.

**What you see.** A row on the Threads screen: which session asks, the query, the account and its reason. Approve it like any
other card. Approving it approves that filter on that account for every session: the next session that asks for the
same query on the same account gets it at once, without a card. Settings > Leases lists the filters you've approved;
revoke one there to need a card again.

**What happens.** With a runner configured for the account (below), the runner creates the filter, which labels what
it catches `Quietdeck/Lease/<tab>` and keeps it out of the inbox and unread. It also moves matching mail already in the
inbox. When the lease ends, the runner deletes the filter; the labelled mail stays where it is for the triage agent
to report. The timeline shows when each filter started and when it was removed.

Gmail won't hold two identical filters, so a session that asks for a filter another session already has shares it.
The filter is removed when the last lease on it ends.

Without a runner, the deck tells the holding session to create the filter itself and register it:

```sh
quietdeck lease register 12 --filter-id ANe1Bmh...
```

and, when the lease ends, to delete it and confirm:

```sh
quietdeck lease cleaned 12
```

If the holder has gone, the fallback tab (`leases.fallback` in `deck.yaml`) is asked instead. A removal nobody
confirms within 15 minutes is marked overdue on the timeline and in Settings > Leases, with a **Mark removed** button
for after you delete the filter in Gmail yourself.

**Require a proposal for every new filter.** This setting is on by default. While it's on, `quietdeck lease acquire`
only works for a filter you approved before; anything new needs `lease propose` and your approval. Switch it off and
sessions can create new filters without asking; each one shows on the timeline as "created a filter without a
proposal" and lands in your approved filters, where you can revoke it.

### The runner's filter tools

The runner creates and deletes filters through your tool gateway, with the tools named under `runner.gmail_filters`
in `node.yaml`. `deploy/node.example.yaml` has a working set for Executor's Gmail API tools:

- `labels` (under `runner`) lists an account's labels, to find the lease label's ID.
- `create_label` creates the label the first time.
- `create` creates the filter. It gets `${lease.query}`, `${lease.label}` and `${lease.label_id}`, and must return the
  filter's `id`.
- `delete` deletes it, with `${lease.filter_id}`. A filter that's already gone counts as deleted.
- `inbox` and `move` (both optional) find matching threads in the inbox and move them, with `${thread_ids}`.

Leave `gmail_filters` out and holders create and remove their own filters, as above.

## Heavy builds

```sh
quietdeck lease acquire heavy-build --reason "release build" --ttl 2h --wait
# … run the build …
quietdeck lease release 14
```

The slot is on your own machine unless you give `--resource <machine>`. With every slot taken, the lease is queued and
`--wait` waits for it; the first in the queue gets the next free slot. Slots per machine are set in `deck.yaml`:

```yaml
leases:
  fallback: { node: server, tab: email triage }
  heavy_build_slots: { laptop: 1, buildbox: 4 }   # 2 for any machine not listed
```

Heavy-build slots need no approval and do nothing outside Quietdeck: they only keep big jobs from running at once.

## Commands

| Command | What it does |
| --- | --- |
| `lease propose <kind> …` | Ask for a lease. A new filter waits on a card. |
| `lease acquire <kind> …` | Take one without a card: a slot, or a filter you approved before. |
| `lease list` | Every current lease, the ones that ended today, and the approved filters. |
| `lease show <id>` | One lease. |
| `lease register <id> --filter-id <id>` | The holder created the filter itself. |
| `lease release <id>` | End your lease. |
| `lease cleaned <id>` | You deleted an ended lease's filter. |

`--ttl` and `--idle` take durations like `30m`, `2h` or `1d`. Agents with the MCP server get the same as tools:
`lease_propose`, `lease_acquire`, `leases`, `lease_register`, `lease_release` and `lease_cleaned`.
