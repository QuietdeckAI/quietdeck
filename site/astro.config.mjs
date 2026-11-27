import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";
import starlightOpenAPI, { openAPISidebarGroups } from "starlight-openapi";
import { site, pages, repo } from "./site.config.mjs";

// Sidebar groups in the order site.config.mjs lists them. Advanced starts collapsed.
const groups = [];
for (const p of pages) {
  let g = groups.find((x) => x.label === p.group);
  if (!g) groups.push((g = { label: p.group, items: [], collapsed: p.group === "Advanced" }));
  g.items.push(p.file);
}

export default defineConfig({
  site: site.url,
  trailingSlash: "always",
  integrations: [
    starlight({
      title: site.title,
      description: site.summary,
      logo: { src: "./src/assets/mark.svg", replacesTitle: false },
      favicon: "/favicon.svg",
      social: [{ icon: "github", label: "GitHub", href: repo }],
      // The API reference is generated from ../api/openapi.json (the repo's copy, so the site matches main), under /api/v1/.
      // scripts/sync-docs.mjs writes .openapi.json from it first (boolean schemas made renderable).
      plugins: [
        starlightOpenAPI([
          {
            base: "api/v1",
            schema: "./.openapi.json",
            sidebar: { label: "API reference", collapsed: true, operations: { badges: true } },
          },
        ]),
      ],
      sidebar: [...groups, ...openAPISidebarGroups],
      customCss: ["./src/styles/docs.css"],
      lastUpdated: false,
      pagination: true,
      // Plain code frames: no drawn terminal window around shell blocks.
      expressiveCode: { defaultProps: { frame: "code" } },
      components: {
        PageTitle: "./src/components/PageTitle.astro",
      },
    }),
  ],
});
