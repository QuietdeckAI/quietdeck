# Quietdeck

Your email, read and sorted before you open it.

An AI agent reads what arrives and writes one short report: what needs you, what it would reply, what can be archived.
You change what you'd do differently and approve. Nothing touches your mail until you do.

**Start with [Quietdeck solo](docs/solo.md)**: one program on your own Mac or Linux computer, with your Gmail (through
[gog](https://github.com/steipete/gogcli)) and Claude Code as the agent. No server, no account, no public address.
[How solo keeps your mail safe](docs/solo-security.md).

**Then, a deck for all your agents**: the full Quietdeck runs a deck on a server of your own and connects agent
sessions on every machine you use. See [Getting started](docs/getting-started.md).

**Docs: [quietdeck.ai](https://quietdeck.ai)** (built from [`docs/`](docs/) by [`site/`](site/)) · for agents:
`llms.txt` on the site.

## Status

Early, built in public, and running one person's email every day. Expect breaking changes.

## Working on Quietdeck

See [AGENTS.md](AGENTS.md): Rust builds run on a build server (`scripts/on-dev`), the web app in `web/` with pnpm.

## Licence

Source-available under the [Functional Source License, Version 1.1, ALv2 Future License](LICENSE.md) (Fair Source):
each release becomes Apache 2.0 two years after it's published.
