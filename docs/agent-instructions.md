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
