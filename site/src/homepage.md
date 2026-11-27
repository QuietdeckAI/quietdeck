# Quietdeck

Your email, read and sorted before you open it.

An AI agent reads what arrives and writes one short report: what needs you, what it would reply, what can be archived.
You change what you'd do differently and approve. Nothing touches your mail until you do. Source-available (Fair
Source, FSL-1.1-ALv2: each release becomes Apache 2.0 after two years), early, built in public:
https://github.com/QuietdeckAI/quietdeck

## How Quietdeck solo works

One program running in the background on a Mac or a Linux computer. Your mail never passes through anyone else's
server.

- **The runner** watches Gmail through gog (https://github.com/steipete/gogcli), a command-line tool you sign in to
  once, and carries out what you approve: archive, label, mark read, save a draft. It's the only part that touches
  your mailbox.
- **The agent** is Claude Code, run for each batch of new mail. It reads only the mail it's given, marked as
  untrusted, and may only post proposals and the report.
- **The Inbox** is a page in your browser, on your computer only, signed in with a one-time link.

## What you get

- **One short report**: what needs you first, numbered; receipts, newsletters and sales pitches grouped at the end under
  one Archive all.
- **Replies drafted, never sent**: saved as Gmail drafts for you to change and send.
- **Say it in your own words**: change any proposal in plain English; the agent updates it and waits for you.
- **Nothing without you**: the agent proposes, you approve. Mail you archive in Gmail yourself drops off the report.
- **Safe with strangers' mail**: https://quietdeck.ai/solo-security.md
- **On your computer**: no server, no account, no public address.

## Start on your own computer

You need a Mac or Linux machine, Gmail, gog and Claude Code. No prebuilt releases yet:

```sh
git clone https://github.com/QuietdeckAI/quietdeck
cd quietdeck && pnpm -C web install && pnpm -C web build
cargo install --locked --path crates/quietdeck-solo
quietdeck-solo init && quietdeck-solo install
quietdeck-solo open
```

The guide: https://quietdeck.ai/solo.md

## Then, a deck for all your agents

Solo is the smallest Quietdeck. The full one runs a deck on a server of your own and connects agents on every machine
you use: forwards from email to the agent that owns the work, agents that message each other, and one place to approve
what all of them propose: https://quietdeck.ai/getting-started.md
