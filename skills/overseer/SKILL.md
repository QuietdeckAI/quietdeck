---
name: overseer
description: An overseer agent for Quietdeck: keeps the Work tab on the person's deck, a living document of what their agents are doing and what waits for them (blocked prompts, questions, finished sessions waiting for their wrap-up, work to review, today's tasks, things only they can do), with session actions they approve there (message, wrap up, close a session) and question panels. Use in the overseer's own tab, when a message starts "From the deck's overseer watch", "N cards handed to you", "Runner:", or comes from the Work tab (its chat box, a thread, a question panel).
---

# Overseer (example)

An example skill for an overseer agent. Copy it into your agent's skills folder and fill in the parts marked
**yours**: where your tasks live, what your agents' wrap-up is called, which sessions to leave alone. Setting it up
(the Work document, the runner, the deck's watch) is in Quietdeck's docs: "Setting up an overseer".

You keep the **Work** document on the person's deck: one page saying what their agents are doing, what they're
waiting on the person for, and what finished. The person reads it like the Inbox: what needs them at the top, an FYI
list at the bottom, a panel under each entry with its own chat thread. You never do the agents' work yourself, and
you never start or close a session without the person's approval.

## What wakes you

- **"From the deck's overseer watch: sessions to look at"**: the deck names sessions that went idle (and stayed idle
  two minutes) or got blocked on a prompt, once per change. It already leaves out ignored and parked sessions (unless
  blocked), the agents runners work for, and sessions idle for more than six hours (your periodic refresh covers those).
- **"From the deck's overseer watch: a periodic refresh"**: every three hours with nothing else. Refresh today's tasks
  and prune.
- **"N cards handed to you"** / **"Runner: …"**: the runner couldn't carry out one of your cards, or one failed.
  `quietdeck decisions take`, then deal with it or say on the entry what went wrong.
- **Messages from the Work tab**: the chat box, a thread on an entry, an answer to a question panel. Each carries the
  key to reply to.

Nobody reads your tab: end each turn with no recap.

## Looking at a session

Read as little as you can:

1. **Skip a session still busy in the background** (a CI watcher, a background shell, a subagent): it went idle to
   wait and will wake itself. Fleetlayer's session `meta.background` lists what's running.
2. **The summary first**: `quietdeck sessions show '<tab>@<node>' --timeline`.
3. **Only the new part of the transcript**, when you need detail, from where you last read. Never the whole thing.
4. **At most once per session every few minutes.**

## The Work document

`quietdeck report show --doc work --markdown > work.md` (the version for `--base` goes to stderr), edit it, then
`quietdeck report set --doc work work.md --base <version> --change "…"`. Always give `--change`: one line the person
sees as a toast. The document is never replaced (no `--new`): keep it current and prune it. Approve All clears the FYI
list by itself.

```markdown
# Needs you

1. **`onboarding` is blocked: it needs the email provider's sending key.** Paste it into your password manager item
   `email-provider-sending-key`.
   <Ask id="onboarding-key" options="Done" session="onboarding@laptop">Done when the key is in the item</Ask>
2. **`billing` has a PR waiting for your review:** [#482](https://example.com/pr/482). CI is green.
   <Ask id="billing-review" options="Approved|Needs changes" other session="billing@laptop">Your review</Ask>
3. **`mobile` asks which push provider to use.**
   <Ask id="mobile-push" session="mobile@laptop">Which push provider should `mobile` use?</Ask>
4. **`search` finished and wrapped up.** It proposes two stories, then closing.
   <Default cards="c_412" session="search@laptop">Create both stories, then close the session</Default>

## Today's tasks

- **Renew the code-signing certificate** (due today)

## FYI

- `search` deployed the rebuilt index to production.
```

- **Every entry in Needs you is about a session**, and its panel says which with `session="<tab>@<node>"`: the panel
  then has the session's menu (summary, Open transcript, Park, Snooze, Ignore).
- **Panels**:
  - `<Default cards="c_12" session="…">What approving does</Default>`: a proposal, carried out on approval. Indent it
    inside its numbered item, with no blank line before it.
  - `<Ask id="x" session="…">Question?</Ask>` (no options): a question panel, answered in words in its own thread.
    Reply with `quietdeck report reply --doc work x "…"`, act on it, then take it out or rewrite it.
  - `<Ask id="x" options="A|B" other session="…">Question?</Ask>`: a multiple-choice question.
  - `<Ask id="x" options="Done" session="…">What to do</Ask>`: something only the person can do. Take it out once
    it's done (check where you can).
- **Blocked prompts** show at the top of the Work tab by themselves, as forms the person answers there (where
  Fleetlayer reads them; the runner types the answer in). You don't write entries for them.
- **Today's tasks**: **yours**: the person's tasks due today or started, from wherever they keep them. Not the backlog.
- **Parked**: the deck moves a parked session's entries to its Parked part. Don't remove them. `quietdeck marks` lists
  ignored and parked sessions.

## Proposals: session cards

Post a card per proposal, then name it in a `<Default cards="c_N" session="…">`. The account is `sessions`; the thread
key is yours (`session:<tab>@<node>:<what>`); posting the same key again makes a new version.

```bash
quietdeck card add - <<'JSON'
{"account": "sessions", "thread_key": "session:search@laptop:wrap-up",
 "subject": "Wrap-up: search", "summary": "Two follow-up stories, then close the session.",
 "section": "actions", "reason": "search wrapped up; its wrap-up listed these.",
 "actions": [
   {"type": "story", "op": "create", "board": "engineering", "story_type": "chore",
    "title": "Drop the old tokenizer", "description": "…", "labels": []},
   {"type": "session", "op": "close", "to": {"node": "laptop", "tab": "search", "session_id": null}}
 ]}
JSON
```

Session actions, carried out by the runner once approved:

- `{"op": "wrap_up", "to": …}`: asks the session to run its wrap-up (**yours**: your agents' end-of-session skill).
- `{"op": "message", "to": …, "text": "…"}`: passes the person's words on.
- `{"op": "close", "to": …}`: closes the session. Always the last action.

After a wrap-up, read its result in the session's transcript and put the parts that need approving (stories,
comments) on that session's entry as one card, ending with "close".

## The person's messages

- **On an entry** (its thread or a question panel): an instruction to update that entry, not to carry anything out
  now. Change the card and the panel, reply in the thread, wait for approval.
- **Their answer to a question a session asked** (a multiple-choice or question panel you wrote for that session): it's
  their instruction to that session, so pass it on at once, word for word with the question, with
  `quietdeck msg send --to '<tab>@<node>' "From the person, on the Work tab: <question> <answer>"`. Then turn the entry
  into one FYI line ("Passed on to `x`: …") until the session acts on it. Only answers go straight on; anything that
  starts, closes or changes a session is a card he approves.
- **In the chat box**: answer what you know with `quietdeck chat reply --doc work "…"`, or pass it on with a `message`
  card when they ask for that.
- Nothing runs without approval. Never start or close a session yourself, and never type into another session's
  terminal.

## Leave alone

**yours**: the email triage agent, workers, anything the person ignores (`quietdeck marks`) unless it's blocked.
