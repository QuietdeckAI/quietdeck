# The Quietdeck API

Every deck has an HTTP API under `/api/v1`: cards, decisions, actions, rules, leases, sessions, the timeline, webhooks
and sign-in. Quietdeck's own web app is one client of it. Use it to build your own frontend, a client library, or a
tool that works with a deck.

The [API reference](/api/v1/) lists every operation, with its parameters, request and response bodies and a code
sample. It's generated from [`api/openapi.json`](https://github.com/QuietdeckAI/quietdeck/blob/main/api/openapi.json),
the OpenAPI 3.1 description in the repository, so this site always matches the main branch. Every deck also serves
the spec it runs at `/api/v1/openapi.json`, so a client can check what its deck supports.

## Licence

The spec is licensed under the [Apache License 2.0](https://github.com/QuietdeckAI/quietdeck/blob/main/api/LICENSE),
separately from the Functional Source License that covers the rest of Quietdeck. A frontend or client you build from
it can use any licence, open or closed.

## Versions and compatibility

`info.version` follows the API, not Quietdeck's releases.

- **Within v1, changes are additive only:** new routes, new optional request fields, new response fields, new enum
  values and new optional query parameters. Each of those raises the minor version (1.0.0 to 1.1.0). A fix to a
  description alone raises the patch version.
- **What a client must tolerate:** response fields it doesn't know (ignore them) and enum values it doesn't know (show
  something neutral, don't fail). Within v1 that's the whole contract: nothing a client already sends or reads stops
  working.
- **A breaking change is a new version:** removing or renaming a route or field, making an optional field required,
  or changing a type or a status code's meaning. Breaking changes go in `/api/v2`, and v1 keeps answering alongside
  it for at least six months after v2 ships.
- **Unversioned `/api/...` paths** answer the same routes, for the nodes, CLIs and web apps that predate v1. They
  aren't part of the published API: new clients use `/api/v1`.

## Sign-in

Each operation names the schemes it accepts:

- **`owner`:** the owner's browser, signed in with a passkey (`/api/v1/auth/login/*`), as a cookie. Requests that
  change something must come from the deck's own origin. Sign-in through Fleetlayer, for frontends served from
  elsewhere, is planned.
- **`session`:** an agent session's token (`Bearer qds_…`), which a node issues to the sessions on it.
- **`node`:** a node's own token (`Bearer qdn_…`), from enrolment.

## What only the default web app uses

Operations tagged **report** or **presence** carry the `x-quietdeck-ui` extension. They belong to the default web
app's own screens: the triage agent's report (its answers, chat, snapshots, and Approve All by its defaults) and
whether the owner is reading the Inbox. A frontend of your own can ignore them and work with cards and decisions
directly, for example `POST /api/v1/decisions/approve-all` with its own list of cards.

## Errors

An error answers `ApiError` (`{"error": "…"}`) with a 4xx or 5xx status:

| Status | Meaning |
|---|---|
| 401 | Not signed in |
| 403 | The wrong kind of caller |
| 404 | Not found |
| 409 | A conflict, or a session that's offline |
| 422 | A request the deck refused |
| 503 | The deck can't do it right now |

A JSON body that doesn't parse at all gets a plain-text 400, 415 or 422 before the deck sees it.

[How Quietdeck works](concepts.md) explains cards, decisions and the rest of what the API works with.
