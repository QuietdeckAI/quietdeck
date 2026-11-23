# Instructions for your agents

Your agents learn how to use Quietdeck from your global instructions: the file every session loads. For Claude Code
that's `~/.claude/CLAUDE.md`; other harnesses read `~/AGENTS.md` or their own. Paste the section below into it on every
machine (or keep the file in sync between machines), and change the triage tab's name to yours.

Quietdeck doesn't write this file for you: it's yours, and you should read every line an agent will follow.

```markdown
# Quietdeck

Sessions on other machines are reachable through Quietdeck. `quietdeck sessions` lists every session on every
machine, as `<tab>@<machine>`.

- **Messages:** on the same machine, use Claude Code's own `SendMessage`. To a session on another machine, use
  `quietdeck msg send --to "<tab>@<machine>" "<text>"` (the machine can be left out when the tab name is unique).
  Message another session only when your task needs it.
- **A send to a session that isn't running fails at once**, and nothing is kept to deliver later. Don't wait for it:
  do the work yourself, read the docs, search, or try again later.
- **A Gmail draft is a starting point.** I edit drafts in Gmail and send them myself, so never assume a draft's text
  is what I sent. Read the sent copy before recording what was promised or quoted.
- **When you finish work that came from an email, tell the triage agent** (the `email triage` tab), with the thread
  ID and what happened, so it can archive the thread or draft the reply. `SendMessage` on the same machine,
  `quietdeck msg send` across machines.
- **Leases.** To keep notification emails out of my inbox while you watch something yourself (CI runs you're
  fixing), ask with `quietdeck lease propose gmail-filter --account <id> --query 'from:<sender> …' --reason "…"`. I
  approve new filters; one I approved before starts at once. Never create a Gmail filter any other way. Before a big
  build or a full test run, take a slot with `quietdeck lease acquire heavy-build --reason "…" --wait`, and
  `quietdeck lease release <id>` when it's done.
- **Pings.** Ping my phone only when I need to know right now and it can't wait a few hours. Use
  `quietdeck ping --title "…" --body "…" --reason "<why it can't wait>" [--link <url>] [--thread <id>]`. Before you
  ping, pick up my feedback on your earlier pings with `quietdeck pings feedback` and follow it.
```

## Why each line is there

- **Two ways to message.** Claude Code already passes messages between sessions on one machine
  ([how it works](claude-code-messaging.md)), so Quietdeck only carries them between machines. Both arrive the same
  way: queued, and read at the session's next turn.
- **Failing at once.** Messages between agents go stale within minutes. A message held until its session came back
  would usually be out of date when it arrived, so the sender hears straight away and finds another way.
- **Drafts.** Drafts are how replies normally leave: the triage agent drafts, you edit and send. An agent that
  recorded the draft's text as what you promised would be wrong whenever you changed it.
- **Work from an email.** The triage agent forwarded the email to the session that does the work; telling it when
  the work is done closes the loop, without the agent watching every session.
- **Pings.** A ping is the only thing that interrupts you, so the bar is high. Your "Don't ping for this again"
  reaches only the agent that pinged, so each agent checks for it before pinging again.
- **Leases.** A filter an agent creates on its own outlives the session and keeps hiding mail nobody reads. A lease
  ends with its session, and approving each new filter once keeps you in charge of what leaves your inbox. Build slots
  stop several sessions from compiling at once and slowing every one of them down.
