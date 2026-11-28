# Quietdeck API

`openapi.json` is the OpenAPI 3.1 description of a Quietdeck deck's API: cards, decisions, actions, rules, leases,
sessions, the timeline, webhooks and sign-in. Every route is under `/api/v1`. Use it to build your own frontend, a
client library or a tool that works with a deck.

- **Licence:** this folder (the spec, this README and the lint config) is licensed under the Apache License 2.0
  (`LICENSE`), not under the Functional Source License that covers the rest of Quietdeck. A frontend or client built
  from the spec can use any licence.
- **A live copy:** every deck serves the spec it runs at `/api/v1/openapi.json`.
- **Generated, not hand-written:** `crates/quietdeck-server/src/openapi.rs` builds the spec from the handlers' own
  request and response types (with [aide](https://github.com/tamasfe/aide) and the types' schemars derives). Its test
  rewrites this file, and CI fails when the committed copy differs from what the code generates or when the spec
  fails `redocly lint`. To update it, run the server's tests (`scripts/on-dev test`) and commit the result.

## Versions and compatibility

`info.version` follows the API, not Quietdeck's releases.

- **Within v1, changes are additive only:** new routes; new optional request fields; new response fields; new enum
  values; new optional query parameters. Each of those raises the minor version (1.0.0 to 1.1.0). A fix to a
  description alone raises the patch version.
- **What a client must tolerate:** response fields it doesn't know (ignore them), and enum values it doesn't know
  (show something neutral, don't fail). Within v1 that's the whole contract: nothing a client already sends or reads
  stops working.
- **A breaking change is a new version:** removing or renaming a route or field, making an optional field required,
  changing a type or a status code's meaning. Breaking changes go in `/api/v2`, and v1 keeps answering alongside it
  for at least six months after v2 ships.
- **Unversioned `/api/...` paths** answer the same routes as `/api/v1/...`, for the nodes, CLIs and web apps that
  predate v1. They're not part of the published API: new clients use `/api/v1`.

## Serving your own frontend

The owner signs in with a passkey, and the session is a cookie on the deck's own origin: a page served from
anywhere else can't act as the owner. Until sign-in for other origins arrives (through Fleetlayer), serve your
frontend from the deck's origin, one of two ways:

- **The deck serves it:** build it to static files and start the deck with `QUIETDECK_UI_DIR=/path/to/dist`. The deck
  serves those files in place of its own web app, and `index.html` for any path that isn't a file (so client-side
  routes work on reload). `/api/...` stays the API. Your pages sign in at `/api/v1/auth/*` like the default app's.
- **A proxy:** serve your pages and pass `/api/` through to the deck on the same origin (a dev server's proxy, a
  reverse proxy in front of both). `examples/inbox` does this with Vite in development.

Agents and scripts don't need either: they send a bearer token (`session` or `node`) from anywhere.

Frontends on other origins come later, designed now (28 November 2026) so nothing has to change when they do: the
deck will take an explicit list of allowed origins in its config, never a wildcard, and send credentials to those
origins only. People will sign in to them through Fleetlayer's sign-in service (OpenID Connect), which the deck
already uses for its own sign-in.

## Making decisions

Two routes, by what the decision does:

- **Approve, edit or adjust: `POST /api/v1/decisions/approve-all`.** These carry work out, so they go through the
  undo window (Settings: `undo_seconds`): the answer, `Approving`, names the batch, and
  `POST /api/v1/decisions/undo` with it cancels everything in it until `release_at`. Send every card on screen, or
  one with `single: true`. Each decision names the card version on screen; a card that changed since is skipped (the
  answer's `skipped` says why).
- **Later, move up, move down: `POST /api/v1/cards/{id}/decide`.** These only move the card, at once, with no undo
  window.

While a decision waits out its window, the card still reads `state: open` and carries `pending` (the decision's kind,
its batch and `release_at`); the Inbox's `approving` names the batch too.

## Live updates

`GET /api/v1/events` is a server-sent event stream. Every event is named `changed` (listen for it by name:
`onmessage` sees only unnamed events), and its data is a JSON string naming what changed (the `Changed` schema:
`"cards"`, `"timeline"`, and so on; `"all"` after the stream fell behind). Refetch that view. The stream ends when the
deck restarts: reconnect and refetch.

## Sign-in

Three security schemes, named on each operation:

- `owner`: the owner's browser, signed in with a passkey (`/api/v1/auth/login/*`), as a cookie. Requests that change
  something must come from the deck's own origin. (Sign-in through Fleetlayer, for frontends served from elsewhere,
  is planned.)
- `session`: an agent's session token (`Bearer qds_…`), which a node issues to the sessions on it.
- `node`: a node's own token (`Bearer qdn_…`), from enrolment.

## What only the default web app uses

Operations tagged `report` or `presence`, which carry the `x-quietdeck-ui` extension, belong to the default web app's
own screens: the triage agent's report (its answers, chat, snapshots and Approve All by its Defaults) and whether the
owner is reading the Inbox. A frontend of its own can ignore them and use the cards and decisions directly, for
example `POST /api/v1/decisions/approve-all` with its own list of cards.

## Errors

An error answers `ApiError` (`{"error": "…"}`) with a 4xx or 5xx status: 401 not signed in, 403 the wrong kind of
caller, 404 not found, 409 a conflict (or a session that's offline), 422 a request the deck refused, 503 the deck
can't do it right now. A request the deck can't read at all (bad JSON, the wrong content type, a bad query) gets
the same `ApiError` shape, with 400, 415 or 422.
