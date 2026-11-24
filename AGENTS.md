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
