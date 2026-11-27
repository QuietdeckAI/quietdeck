# @quietdeck/client

A small typed client for a Quietdeck deck's API (`/api/v1`). It's a thin layer over
[openapi-fetch](https://openapi-ts.dev/openapi-fetch/), and its types are generated from `api/openapi.json` by
[openapi-typescript](https://openapi-ts.dev/), so paths, request bodies and answers are checked at compile time.

Licensed under the Apache License 2.0 (`LICENSE`), like the spec: build a frontend or tool on it under any licence.
It isn't published to npm yet. Copy this folder, or depend on it from a checkout.

```ts
import { createQuietdeckClient, type Schemas } from "@quietdeck/client";

// In a page the deck serves, signed in with a passkey: no options needed.
const deck = createQuietdeckClient({ baseUrl: "https://deck.example.com" });

const { data: inbox, error } = await deck.GET("/api/v1/inbox", {});
if (error) throw new Error(error.error);

const cards: Schemas["Card"][] = inbox.actions;
await deck.POST("/api/v1/decisions/approve-all", {
  body: { cards: cards.map((c) => ({ card_id: c.id, version: c.version, kind: "approve" })) },
});
```

An agent or a script passes a bearer token instead: `createQuietdeckClient({ baseUrl, token: "qds_…" })`.

- **Compatibility:** the API changes only additively within v1 (`api/README.md`). A newer deck may answer fields
  and enum values this client's types don't name yet. Treat unknown enum values as something neutral.
- **What only the default web app uses:** operations tagged `report` or `presence` (with `x-quietdeck-ui`).
- **Regenerating:** `pnpm install && pnpm generate` after the spec changes. CI runs `pnpm check`, which fails
  when `src/schema.ts` differs from what the spec generates.
