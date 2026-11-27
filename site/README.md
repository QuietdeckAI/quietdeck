# quietdeck.ai

The site: a homepage and the docs, built with [Astro](https://astro.build) and
[Starlight](https://starlight.astro.build) from the repo's `docs/` folder, the one source of truth. Adapted from
Fleetlayer's site (Apache 2.0): see [NOTICE](NOTICE).

```sh
cd site
npm ci
npm run dev      # http://localhost:4321
npm run build    # dist/
npm run check    # fails on broken internal links
```

- **A new doc** goes in `docs/` as plain Markdown starting with a `# Title` line (no frontmatter), and gets a line in
  `site.config.mjs` (its sidebar group). The build warns about any doc that isn't listed there or in `unpublished`.
- **Links between docs** stay relative (`solo-security.md`), so they work on GitHub too; the build rewrites them.
- **Every page is also Markdown** at its address plus `.md`, and `/llms.txt` and `/llms-full.txt` list and hold them
  all (`scripts/sync-docs.mjs`).
- **The homepage** is `src/pages/index.astro`, and its Markdown twin for agents is `src/homepage.md`: change both.
- **Not deployed yet.** `wrangler.jsonc` is ready for Cloudflare Workers (static assets), as Fleetlayer's site does it,
  but there's no deploy job until the domain and its token are set up.
- **Wording:** Quietdeck is "source-available" or "Fair Source" (FSL-1.1-ALv2), never "open source".
- No analytics, trackers or third-party scripts; fonts are self-hosted.
