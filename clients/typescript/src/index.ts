// A typed client for a Quietdeck deck's API (/api/v1). Licensed under the Apache License 2.0 (LICENSE).
//
// `schema.ts` is generated from api/openapi.json (`pnpm generate`); CI fails when it differs from the spec.
// Paths, bodies and answers are all typed from it:
//
//   const deck = createQuietdeckClient({ baseUrl: "https://deck.example.com" });
//   const { data, error } = await deck.GET("/api/v1/inbox", { params: { query: { account: "work" } } });
//   await deck.POST("/api/v1/cards/{id}/decide", { params: { path: { id: 42 } }, body: { version: 3, kind: "later" } });

import createClient, { type Client, type Middleware } from "openapi-fetch";

import type { components, operations, paths } from "./schema.ts";

export type { components, operations, paths };

/** Every schema in the spec by name: `Schemas["Card"]`, `Schemas["Inbox"]`. */
export type Schemas = components["schemas"];

/** The version of the API this client was generated from (the spec's `info.version`). */
export const API_VERSION = "1.10.0";

export interface QuietdeckClientOptions {
  /** The deck's address, without `/api/v1`: `https://deck.example.com`. Leave empty in a page the deck serves. */
  baseUrl?: string;
  /**
   * A bearer token: an agent's session token (`qds_…`) or a node's (`qdn_…`). Leave it out in the owner's browser,
   * which signs in with a passkey and sends its cookie.
   */
  token?: string;
  /** Your own `fetch` (tests, a server runtime). */
  fetch?: typeof fetch;
}

export type QuietdeckClient = Client<paths>;

/** A client for one deck. Errors answer `{ error: Schemas["ApiError"] }` with the response's status. */
export function createQuietdeckClient(options: QuietdeckClientOptions = {}): QuietdeckClient {
  const client = createClient<paths>({
    baseUrl: options.baseUrl ?? "",
    fetch: options.fetch,
    // The owner's session is an HttpOnly cookie on the deck's own origin.
    credentials: options.token ? "omit" : "same-origin",
  });
  if (options.token) {
    const token = options.token;
    const auth: Middleware = {
      onRequest({ request }) {
        request.headers.set("Authorization", `Bearer ${token}`);
        return request;
      },
    };
    client.use(auth);
  }
  return client;
}
