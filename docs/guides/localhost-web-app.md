# A web app on localhost isn't safe by itself

A tool that serves a web page on `127.0.0.1` feels private: nothing outside your computer can reach it. But your
browser can, and your browser runs code from every website you visit. Any of those pages can send requests to
`http://127.0.0.1:<port>`, and with a little more work read the answers. This guide is for anyone building a local web
app that does something on the person's behalf: an agent dashboard, a dev tool with buttons, an approval page.
Quietdeck solo's Inbox is one of those (it archives mail and saves drafts when you approve), and has no passkeys, so it
had to be safe some other way. Here's what it checks, and why each check is there.

## What a web page can do to your localhost

- **Send a request that acts (CSRF).** A hidden form that posts to `http://127.0.0.1:7350/api/...`, or a `fetch` with
  `mode: "no-cors"`, goes through. The page can't read the answer, but the approval or the archive has already
  happened. If the app trusts requests because they come from this computer, that's all it takes.
- **Read the answers (DNS rebinding).** A page on `attacker.example` makes its own name resolve to `127.0.0.1` a moment
  after loading. Its requests to `http://attacker.example:7350` now reach your app, and since that's the page's own
  origin, the browser lets it read every answer.
- **Look like you.** Every request to a loopback app comes from `127.0.0.1`: yours, the ones other websites make your
  browser send, and every other process on the machine. The address says nothing about who's asking.

A `SameSite` cookie helps less than it looks: sites are compared by domain, not port, so a page served by any other
app on `localhost` (a dev server, another tool) counts as the same site as yours.

Quietdeck solo answers each of these in one piece of middleware that every request passes first.

## Listen on loopback, and refuse anything else

The deck refuses to start on an address that isn't loopback, and that includes the name `localhost`, which could
resolve anywhere:

```rust
let addr: SocketAddr = listen.parse()?;     // "127.0.0.1:7350" or "[::1]:7350", never a name
ensure!(addr.ip().is_loopback(), "listens only on this machine, never on {listen}");
```

This keeps other machines out. It does nothing about the browser on this one.

## Check the Host header

A rebinding page's requests carry its own name in `Host` (`attacker.example:7350`). The deck answers only requests
addressed to its own loopback address and port, before anything else, signed in or not:

```rust
// HTTP/1.1 sends Host; HTTP/2 puts it in the request's authority.
let host = req.headers().get(HOST).and_then(|v| v.to_str().ok())
    .map(str::to_string)
    .or_else(|| req.uri().authority().map(|a| a.to_string()))
    .unwrap_or_default();
let own = [format!("127.0.0.1:{port}"), format!("localhost:{port}"), format!("[::1]:{port}")];
if !own.contains(&host) {
    return refuse(421, "this deck answers only at its own address on this machine");
}
```

A missing Host is refused too, and so is the right address on the wrong port.

## Sign the browser in with a one-time link

Nothing is trusted by address, so a browser has to sign in. With no passkeys and no password, solo uses a link that
works once: `quietdeck-solo open` writes a random code (24 bytes) to a file in the deck's data folder, which only your
user can open, and opens `http://127.0.0.1:7350/solo/login?code=<code>` in your browser.

- The file is named by the code's SHA-256 hash and holds its expiry, five minutes ahead. The code itself is never
  stored.
- Visiting the link deletes the file, and only a visit that managed to delete it signs in, so two visits can't both
  succeed.
- A signed-in browser gets a session cookie (a random 32-byte token, kept only as its hash) with `HttpOnly` and
  `SameSite=Strict`. A session that goes unused for 30 days ends.

Whoever can run `quietdeck-solo open` or read that folder is already you, so the link proves no more than it should.
Every other request without a valid session gets a 401 that says to run `quietdeck-solo open`, except the health
checks and the login link itself.

## Changes come only from your own pages

The session cookie stops a page that doesn't have it, but a browser that's signed in sends it whenever it can. So
anything that changes state (any method but GET, HEAD and OPTIONS) must also say it came from the deck's own pages:

```rust
let own = format!("http://{host}");
let from_own_pages = match (origin, referer) {
    (Some(o), _) => o.trim_end_matches('/') == own,      // "null" doesn't match
    (None, Some(r)) => r == own || r.starts_with(&format!("{own}/")),
    (None, None) => false,
};
if changes && !from_own_pages {
    return refuse(403, "this request didn't come from the deck's own pages");
}
```

- Origin is checked first. Browsers send it on cross-origin requests and on most POSTs; Referer covers the ones that
  don't.
- The Referer has to be the origin itself or the origin followed by `/`, so `http://127.0.0.1:7350.attacker.example/`
  doesn't pass a prefix check.
- A change with neither header is refused. Your own pages always send one; a request that hides both has a reason to.
- This only works if GET never changes anything. The login link is a GET that does, which is why its code is single
  use and short-lived, and a cross-site POST to it is refused like any other.

## Tokens, and no CORS

The `quietdeck` command and the runner talk to the same deck with a Bearer token, not a cookie. Requests with an
`Authorization` header skip the session check and go on to the usual token checks, which never trust a request by
address and never make a token the owner. The CLI gets its token from a socket only your user can open.

That's safe because of what browsers won't do: a page can't send an `Authorization` header to another origin without
first asking with a CORS preflight, and the deck answers no preflight with an `Access-Control-Allow-*` header. So the
browser never sends the real request. Keep it that way: a permissive CORS layer added to get some local page working
would let any website call the API with a token it found.

## Passkeys won't take an IP address

If you add passkeys to a local app later, there's a catch: WebAuthn's relying party ID must be a domain name, and
`webauthn-rs` refuses an IP address outright. Quietdeck's deck sets up passkeys from its public URL when it starts, so a
public URL of `http://127.0.0.1:7350` stops it starting. Solo gives the deck `http://localhost:<port>` as its public
URL, while its links and the address it listens on use `127.0.0.1`; the Host check accepts both. A passkey is tied to
the name it was made at, so one made at `localhost` won't work at `127.0.0.1`.

## Test every refusal

Each check is easy to break in a refactor and silent when it's broken, so each has a test against the middleware:

- a foreign Host, another port and no Host are refused, even with a valid session;
- no session, or a made-up cookie, gets a 401, and nothing is trusted by address;
- a login link works once, a used, guessed or expired one doesn't, and the session outlives a restart;
- a change with a foreign Origin, `Origin: null`, no Origin and no Referer, or a lookalike Referer is refused;
- a preflight from another origin gets no `Access-Control-*` headers.

## Where this lives in Quietdeck

- The middleware, the one-time links, the sessions and their tests:
  [crates/quietdeck-server/src/solo.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-server/src/solo.rs)
- The loopback check before anything binds, and the middleware wrapped around every route:
  [crates/quietdeck-server/src/lib.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-server/src/lib.rs)
- Passkeys refusing an IP address:
  [crates/quietdeck-server/src/passkeys.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-server/src/passkeys.rs)
- `quietdeck-solo open` and the `localhost` public URL:
  [crates/quietdeck-solo/src/main.rs](https://github.com/QuietdeckAI/quietdeck/blob/main/crates/quietdeck-solo/src/main.rs)

[How solo keeps your mail safe](../solo-security.md) covers the rest of solo's protections.
