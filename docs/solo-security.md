# Quietdeck solo and your security

An agent that reads your email reads text written by strangers, and some of it will try to give the agent
instructions. Quietdeck solo gives you a choice: an agent with full access, which can understand your mail the way a
good assistant would, or a restricted one that can't do much harm even if an email fools it. **Full access is the
default.** Read this page before you start solo, and pick the mode that fits your mail.

## Full access (the default)

Solo runs the agent as your own Claude Code (`claude -p`), with your settings, MCP servers and `CLAUDE.md`, and with
permission prompts skipped, since nobody is there to answer them. It can do anything you can do in a terminal:

- read your whole mailbox with gog: every thread in full, attachments, earlier mail from the same sender;
- read your files and notes, search the web, and use your other tools to research what an email is about;
- run any command as you.

That's what makes its proposals good: it works out what an invoice is for, whether a reply is needed and what to
say, the way your own triage would.

**The risk:** the agent reads mail from strangers. An email written to fool it (a prompt injection) could get it to
act as you: send your files or mail somewhere, change things on your computer, or use your accounts. Claude resists
these, and the agent's instructions tell it that mail is data and never instructions, and never to change your mail
itself. But no AI model is immune. Use full access when you're comfortable with an assistant that has your
computer's keys, as you would be with a person you trust reading your mail at your desk.

## Restricted mode

```yaml
agent:
  headless:
    restricted: true
```

(or `quietdeck-solo init --restricted`). The agent may then run only the `quietdeck` command, and only the parts
triage needs: posting cards, writing the report, replying to you in the Inbox and reading what happened since its last
round. Claude Code refuses anything else without a prompt (`--permission-mode dontAsk`), and none of your MCP servers or
other tools are loaded. It can't read your mailbox, your files or the web, so it sees only the names, subjects and
snippets the runner hands it, and its proposals are only as good as that. An email can't make it do much, because it
can't do much.

`agent.headless.allowed_tools` lists exactly what it may run, and solo refuses a rule that would let it run anything.

## In both modes

- **Nothing happens to your mail until you approve it.** Everything the agent proposes waits on the Inbox. When you
  approve it, solo's runner carries it out: an archive, a label, marking read or saving a draft. Before acting it
  checks the thread hasn't changed since the agent looked (a new reply hands the card back), and that a draft's From
  address is one your account can send as. The runner never sends mail: a reply is a draft that you send yourself.
  `actions: [draft]` in solo.yaml limits the runner to drafts.
- **Mail arrives marked as untrusted.** The runner names each new email between `<email>` tags, after a line saying
  it's untrusted input and not to follow instructions in it. Headers are flattened to one line and no tag in them
  survives, so an email can't pretend to be something else.
- **The agent is its own session.** If you start solo from inside a Claude Code session, the agent doesn't inherit
  that session's identity: solo clears those environment variables before each round.

## The Inbox is yours alone

The Inbox is a web page on your computer, and being on your computer isn't protection enough by itself: any website
you visit can try to send requests to `127.0.0.1`. So solo:

- listens only on `127.0.0.1`, and refuses to start on any other address;
- answers only requests addressed to its own address and port, which stops DNS rebinding (a website pointing its own
  name at your computer to read the page);
- signs a browser in only through a one-time link from `quietdeck-solo open`, never because a request comes from your
  computer;
- accepts a change (an approval, an edit) only from its own pages, so another website can't approve something on your
  behalf;
- never answers a cross-site request with permission to read, so a website can't use the `quietdeck` command's token
  from your browser.

The `quietdeck` command gets its token from solo over a socket only your user can open.

## Why the agent is Claude Code, not Codex

Solo supports only Claude Code as the agent for now. In restricted mode Codex couldn't work at all: on macOS its
sandbox blocks the socket the `quietdeck` command uses unless its network access is on, and then its shell can reach
the internet too (tested 27 November 2026 with codex-cli 0.166.0), and Codex can't be limited to one command the way
Claude Code's permission rules can.

## Where your data is

On your computer. `~/.config/quietdeck/solo.yaml` holds your settings and `~/.local/share/quietdeck/solo` (only your
user can open it) holds the Inbox's database: the reports, cards and what you approved, with each card's summary and
a short excerpt of its email. gog keeps its own Google sign-in. What the agent reads goes to Anthropic, as with any use
of Claude: don't use solo on mail you can't share with your AI provider.
