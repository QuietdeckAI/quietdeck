# Quietdeck

Quietdeck is source-available under FSL-1.1-ALv2 (`LICENSE.md`): each release becomes Apache 2.0 two years after
it's published. Call it "source-available" or "Fair Source", never "open source".

## Code from other projects

- **Apache 2.0 and MIT projects** (herdr, herdr-core, herdr-app): code can be borrowed. Keep their copyright notices
  and NOTICE file, and mark files you change.
- **GPL and AGPL projects** never go into Quietdeck, not even a snippet. `herdr-mobile-relay` (`~/code/herdr-mobile-relay`)
  is AGPL-3.0. Quietdeck's relay is our own design, not a port: when writing Quietdeck code, don't read anything under
  `~/code/herdr-mobile-relay`.
- Check a project's licence before taking anything from it, and ask Nathan about any other copyleft licence.

## Builds and tests run on docspring-dev

Rust builds, tests and clippy run on docspring-dev, the home server, never on a Mac: the workspace's `target/` grew
past 50 GB on Nathan's Mac and filled its disk. Use `scripts/on-dev test`, `scripts/on-dev clippy`,
`scripts/on-dev check` and `scripts/on-dev build` (cargo arguments go after the command). `test` copies back the
TypeScript that ts-rs writes to `web/src/types`. The web app (pnpm, Vite, Playwright) runs locally. The one Rust build
on a Mac is the dev deck's server, whose data stays there: `scripts/dev-deck-build` builds only that, without debug
info, in `~/.cache/quietdeck-dev-deck`.

Release images build on docspring-dev too: `scripts/build-images --wait <commit>` pushes
`harbor.home.ndbroadbent.com/quietdeck/deck:<sha>` and `node-bin:<sha>`, which the homeserver session deploys.
A Mac has no release binary: its node is built on the Mac itself with `scripts/install-node-from-source <commit>`
(release build only, in `~/.cache/quietdeck-node-build`, a temporary worktree), the one other Rust build allowed there.

## Guides to write

Every time you run into something others will hit too, document it for the people who follow in our footsteps: a
setup step, a footgun (a prompt that blocks unattended sessions, a config that silently does nothing), a safety rule, a
design reason. This is a rule, not a nicety (Nathan, 25 November 2026).

1. Add an entry to `docs/guides-to-write.md` in the same change, saying what it is, how it was solved and where the
   knowledge lives now (a file, a commit, a runbook).
2. Send it to the planning session (`quietdeck msg send --to "planning@nathans-mac"`): the problem, the fix, how it
   was tested, and what's still open. Planning gathers it for Quietdeck's docs and for MailExec's, whose users run
   agents on their own machines too.

Do it when the problem is solved, not later: fast sessions move on, and nothing may fall through the cracks before the
guides are written.
