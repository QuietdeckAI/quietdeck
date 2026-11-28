//! Signing the owner in through Fleetlayer's sign-in service (OpenID Connect, code flow with PKCE), beside passkeys.
//! Fleetlayer's own client does the protocol (`fleetlayer-oidc`, as pages and its Claude sign-in hub use it); the deck
//! adds the owner check and its browser session, the same cookie a passkey sign-in makes.
//!
//! - `GET /auth/signin?next=<path>` sends the browser to the sign-in service, with the state in a short-lived cookie.
//! - `GET /auth/callback` checks the answer, refuses anyone but the configured owner, and starts a browser session.
//!   It answers with a page that moves on, because the session cookie is `SameSite=Strict` and a browser wouldn't send
//!   it on a redirect that is still the sign-in service's cross-site navigation.
//! - `POST /auth/backchannel-logout` ends the sessions a logout token names (signing out at the sign-in service, or
//!   "sign out everywhere"), checked by the client (signature, issuer, audience, age, event, no nonce, unseen `jti`).
//!
//! Register the deck with the sign-in service as client `quietdeck` (or QUIETDECK_FLEETLAYER_SIGNIN_CLIENT), with
//! `redirect_uris: [<deck>/auth/callback]`, `post_logout_redirect_uris: [<deck>/]` and
//! `backchannel_logout_uri: <deck>/auth/backchannel-logout`.

use axum::Router;
use axum::extract::{Form, Query, State};
use axum::http::header::{COOKIE, SET_COOKIE, USER_AGENT};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use fleetlayer_oidc::{Oidc, Person, continue_page};
use serde::Deserialize;

use crate::auth::{random_bytes, sha256_hex};
use crate::config::Env;
use crate::db::now_ms;
use crate::state::AppState;

/// The cookie that binds a sign-in to the browser that started it.
const STATE_COOKIE: &str = "qd_signin_state";

pub struct Signin {
    oidc: Oidc,
    /// The owner's handle or person ID at the sign-in service.
    owner: String,
    secure: bool,
}

impl Signin {
    pub fn new(issuer: &str, client: &str, secret: &str, public_url: &str, owner: &str) -> anyhow::Result<Signin> {
        let base = public_url.trim_end_matches('/');
        let oidc = Oidc::with_redirect(issuer, client, secret, &format!("{base}/auth/callback"), &format!("{base}/"))?;
        anyhow::ensure!(!owner.trim().is_empty(), "QUIETDECK_FLEETLAYER_SIGNIN_OWNER is empty");
        Ok(Signin { oidc, owner: owner.trim().to_string(), secure: public_url.starts_with("https://") })
    }

    /// On when the sign-in service's address, the client secret's file and the owner are all set. Some set without
    /// the rest is a mistake worth stopping for, except the address alone, which the passkey step-up uses by itself.
    pub fn from_env(env: &Env) -> anyhow::Result<Option<Signin>> {
        let (url, secret_file, owner) =
            (&env.fleetlayer_signin_url, &env.fleetlayer_signin_secret_file, &env.fleetlayer_signin_owner);
        match (url, secret_file, owner) {
            (_, None, None) => Ok(None),
            (Some(url), Some(file), Some(owner)) => {
                let secret = std::fs::read_to_string(file).map_err(|e| {
                    anyhow::anyhow!("reading QUIETDECK_FLEETLAYER_SIGNIN_SECRET_FILE {}: {e}", file.display())
                })?;
                let client = env.fleetlayer_signin_client.as_deref().unwrap_or("quietdeck");
                tracing::info!(issuer = %url, %client, "the owner can sign in through Fleetlayer's sign-in service");
                Ok(Some(Signin::new(url, client, &secret, &env.public_url, owner)?))
            }
            _ => anyhow::bail!(
                "signing in through Fleetlayer needs QUIETDECK_FLEETLAYER_SIGNIN_URL, QUIETDECK_FLEETLAYER_SIGNIN_SECRET_FILE \
                 and QUIETDECK_FLEETLAYER_SIGNIN_OWNER together"
            ),
        }
    }

    /// Whether this person is the deck's owner.
    fn is_owner(&self, p: &Person) -> bool {
        p.sub == self.owner || p.handle.as_deref() == Some(self.owner.as_str())
    }

    fn state_cookie(&self, value: &str, max_age: u32) -> HeaderValue {
        // Lax, not Strict: it must come back with the sign-in service's redirect to the callback.
        let secure = if self.secure { "; Secure" } else { "" };
        HeaderValue::from_str(&format!(
            "{STATE_COOKIE}={value}; Path=/auth/callback; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}"
        ))
        .expect("the cookie is ASCII")
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/auth/signin", get(start))
        .route("/auth/callback", get(callback))
        .route("/auth/backchannel-logout", post(backchannel_logout))
}

#[derive(Deserialize)]
struct Next {
    next: Option<String>,
}

/// Only a path on the deck itself: never `//elsewhere` or a full URL.
fn safe_next(next: Option<&str>) -> String {
    match next {
        Some(n) if n.starts_with('/') && !n.starts_with("//") && !n.contains('\\') => n.to_string(),
        _ => "/".to_string(),
    }
}

fn not_configured() -> Response {
    (StatusCode::NOT_FOUND, "this deck doesn't sign in through Fleetlayer").into_response()
}

async fn start(State(state): State<AppState>, Query(q): Query<Next>) -> Response {
    let Some(s) = state.signin.clone() else { return not_configured() };
    match s.oidc.start(&safe_next(q.next.as_deref())).await {
        Ok((url, bind)) => {
            let mut res = Redirect::to(&url).into_response();
            res.headers_mut().insert(SET_COOKIE, s.state_cookie(&bind, 600));
            res
        }
        Err(e) => {
            tracing::warn!(error = %e, "starting a sign-in through Fleetlayer");
            (StatusCode::BAD_GATEWAY, format!("Fleetlayer's sign-in service didn't answer: {e}")).into_response()
        }
    }
}

#[derive(Deserialize)]
struct Callback {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get_all(COOKIE).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(';')).find_map(|pair| {
        let (n, value) = pair.trim().split_once('=')?;
        (n == name && !value.is_empty()).then(|| value.to_string())
    })
}

async fn callback(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<Callback>) -> Response {
    let Some(s) = state.signin.clone() else { return not_configured() };
    if let Some(e) = q.error {
        let why = q.error_description.unwrap_or(e);
        return (StatusCode::FORBIDDEN, format!("Fleetlayer's sign-in didn't let you in: {why}")).into_response();
    }
    let (Some(code), Some(st)) = (q.code, q.state) else {
        return (StatusCode::BAD_REQUEST, "the sign-in came back without a code").into_response();
    };
    let bound = cookie(&headers, STATE_COOKIE);
    let (person, next) = match s.oidc.finish(&code, &st, bound.as_deref()).await {
        Ok(done) => done,
        Err(e) => {
            tracing::warn!(error = %e, "finishing a sign-in through Fleetlayer");
            return (StatusCode::FORBIDDEN, format!("the sign-in didn't check out: {e}. Try again from the deck."))
                .into_response();
        }
    };
    if !s.is_owner(&person) {
        tracing::warn!(sub = %person.sub, handle = ?person.handle, "a sign-in through Fleetlayer by someone who isn't the owner");
        return (
            StatusCode::FORBIDDEN,
            format!(
                "{} isn't this deck's owner. A deck has one owner (QUIETDECK_FLEETLAYER_SIGNIN_OWNER).",
                person.handle.as_deref().unwrap_or(&person.sub)
            ),
        )
            .into_response();
    }
    let token = B64.encode(random_bytes::<32>());
    let now = now_ms();
    let device: String =
        headers.get(USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("").chars().take(200).collect();
    let saved = sqlx::query(
        "INSERT INTO auth_sessions (token_hash, user_id, device, created_at, last_seen_at, verified_at, signin_sub, \
         signin_sid, signin_handle) VALUES (?, 1, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(sha256_hex(&token))
    .bind(device)
    .bind(now)
    .bind(now)
    .bind(now)
    .bind(&person.sub)
    .bind(&person.sid)
    .bind(&person.handle)
    .execute(&state.db)
    .await;
    if let Err(e) = saved {
        tracing::error!(error = %e, "saving a browser session");
        return (StatusCode::INTERNAL_SERVER_ERROR, "the deck couldn't save your session").into_response();
    }
    tracing::info!(sub = %person.sub, handle = ?person.handle, "the owner signed in through Fleetlayer");
    let mut res = Html(continue_page(&next, "Signed in", "Continue to Quietdeck")).into_response();
    let h = res.headers_mut();
    h.append(SET_COOKIE, crate::passkeys::set_cookie(&state, &token));
    h.append(SET_COOKIE, s.state_cookie("", 0));
    res
}

#[derive(Deserialize)]
struct Logout {
    logout_token: String,
}

async fn backchannel_logout(State(state): State<AppState>, Form(f): Form<Logout>) -> Response {
    let Some(s) = state.signin.clone() else { return not_configured() };
    let out = match s.oidc.verify_logout(&f.logout_token).await {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!(error = %e, "a back-channel logout that didn't check out");
            return (StatusCode::BAD_REQUEST, format!("{e}")).into_response();
        }
    };
    let ended = match (&out.sid, &out.sub) {
        (Some(sid), Some(sub)) => {
            sqlx::query("DELETE FROM auth_sessions WHERE signin_sid = ? AND signin_sub = ?")
                .bind(sid)
                .bind(sub)
                .execute(&state.db)
                .await
        }
        (Some(sid), None) => {
            sqlx::query("DELETE FROM auth_sessions WHERE signin_sid = ?").bind(sid).execute(&state.db).await
        }
        (None, Some(sub)) => {
            sqlx::query("DELETE FROM auth_sessions WHERE signin_sub = ?").bind(sub).execute(&state.db).await
        }
        (None, None) => return (StatusCode::BAD_REQUEST, "the logout names no session").into_response(),
    };
    match ended {
        Ok(r) => {
            tracing::info!(sessions = r.rows_affected(), "signed out by Fleetlayer's sign-in service");
            // The spec asks for no caching of the answer.
            ([(axum::http::header::CACHE_CONTROL, "no-store")], StatusCode::OK).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "ending sessions for a back-channel logout");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use axum::Json;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use super::*;

    /// A throwaway RSA key for these tests only; it signs nothing anywhere else.
    const KEY: &[u8] = include_bytes!("../tests-data/signin-test-only-key.pem");
    const N: &str = "zv0Qvo4Ow5Zjq9Ls1RY8ota9mX2fqlX2xhIEAPdYjNCEqeDgVhhAYlG8DZJkQTxY5ybOvji9STtZfpcv4bN2HomZd52kelIuZxhg5Hh5IK-AWvXYG0Gu8mzsbjLPIUxqsfv0kM-TZeLG-dvHPV6PU-xl4jw8uIK--8Vyy-BUjooFbaxDlc3jyZffsdFmnv0hBOyAW1b962az-rkpGPvT3tpjYdY0qbwwZPm0vYIWW7kJiy9284AGSS1jQp3xwyqTxQ1CQn4aO6IohfXEAqichFgOYTu6KJ2c538u5vw2K7L2jvhKUeteyZf8k-O2uPOYeBWTFtYT7Q0VsnwtJo8c5w";

    /// A sign-in service for the tests: discovery, its JWKS, and a token endpoint answering with the ID token set.
    struct Issuer {
        url: String,
        token: Mutex<Option<String>>,
    }

    impl Issuer {
        async fn start() -> Arc<Issuer> {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
            let me = Arc::new(Issuer { url, token: Mutex::default() });
            let m = me.clone();
            let router = Router::new()
                .route(
                    "/.well-known/openid-configuration",
                    get({
                        let m = m.clone();
                        move || async move {
                            Json(json!({"issuer": m.url, "authorization_endpoint": format!("{}/authorize", m.url),
                                "token_endpoint": format!("{}/token", m.url), "jwks_uri": format!("{}/jwks", m.url)}))
                        }
                    }),
                )
                .route(
                    "/jwks",
                    get(|| async {
                        Json(json!({"keys": [{"kty": "RSA", "alg": "RS256", "use": "sig", "kid": "k1", "n": N, "e": "AQAB"}]}))
                    }),
                )
                .route(
                    "/token",
                    post(move || {
                        let m = m.clone();
                        async move { Json(json!({"id_token": m.token.lock().unwrap().clone(), "token_type": "Bearer"})) }
                    }),
                );
            tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
            me
        }

        fn sign(&self, typ: &str, claims: &Value) -> String {
            let mut h = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
            h.kid = Some("k1".into());
            h.typ = Some(typ.into());
            jsonwebtoken::encode(&h, claims, &jsonwebtoken::EncodingKey::from_rsa_pem(KEY).unwrap()).unwrap()
        }

        fn id_token(&self, nonce: &str, sub: &str, handle: &str) -> String {
            let now = jiff::Timestamp::now().as_second();
            self.sign(
                "JWT",
                &json!({"iss": self.url, "aud": "quietdeck", "sub": sub, "exp": now + 600, "iat": now,
                "nonce": nonce, "preferred_username": handle, "sid": "sid-1", "fleetlayer.dev/tenant": "home"}),
            )
        }

        fn logout_token(&self, sub: &str, sid: &str) -> String {
            let now = jiff::Timestamp::now().as_second();
            self.sign(
                "logout+jwt",
                &json!({"iss": self.url, "aud": "quietdeck", "iat": now, "exp": now + 120,
                "jti": format!("j-{now}-{sid}"), "sub": sub, "sid": sid,
                "events": {"http://schemas.openid.net/event/backchannel-logout": {}}}),
            )
        }
    }

    fn param(url: &str, name: &str) -> String {
        let q = url.split_once('?').unwrap().1;
        let all: HashMap<String, String> = serde_urlencoded_pairs(q);
        all[name].clone()
    }

    fn serde_urlencoded_pairs(q: &str) -> HashMap<String, String> {
        q.split('&').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.to_string(), v.replace("%20", " "))).collect()
    }

    async fn deck(f: &Issuer, owner: &str) -> AppState {
        let mut state = AppState::for_tests().await;
        // The owner's row, which `serve` makes at start.
        sqlx::query("INSERT INTO users (id, name, tailscale_login) VALUES (1, 'owner', 'owner')")
            .execute(&state.db)
            .await
            .unwrap();
        state.signin = Some(Arc::new(Signin::new(&f.url, "quietdeck", "s3cret", "http://deck.test", owner).unwrap()));
        state
    }

    async fn call(state: &AppState, req: Request<Body>) -> Response {
        router().with_state(state.clone()).oneshot(req).await.unwrap()
    }

    /// Start a sign-in, then come back with an ID token naming this person. `with_state_cookie: false` comes back in a
    /// browser that didn't start it.
    async fn sign_in(state: &AppState, f: &Issuer, sub: &str, handle: &str, with_state_cookie: bool) -> Response {
        let res = call(state, Request::get("/auth/signin?next=/work").body(Body::empty()).unwrap()).await;
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        let to = res.headers()["location"].to_str().unwrap().to_string();
        assert_eq!(param(&to, "code_challenge_method"), "S256");
        let bind = res.headers()[SET_COOKIE].to_str().unwrap().split(';').next().unwrap().to_string();
        *f.token.lock().unwrap() = Some(f.id_token(&param(&to, "nonce"), sub, handle));
        let back = format!("/auth/callback?code=c1&state={}", param(&to, "state"));
        let mut req = Request::get(back);
        if with_state_cookie {
            req = req.header(COOKIE, bind);
        }
        call(state, req.body(Body::empty()).unwrap()).await
    }

    fn session_cookie(res: &Response) -> Option<String> {
        res.headers().get_all(SET_COOKIE).iter().filter_map(|v| v.to_str().ok()).find_map(|v| {
            let pair = v.split(';').next()?;
            (pair.starts_with("qd_auth=") && pair.len() > "qd_auth=".len()).then(|| pair.to_string())
        })
    }

    async fn signed_in(state: &AppState, cookie: &str) -> Option<crate::passkeys::SignedIn> {
        let (parts, _) = Request::get("/").header(COOKIE, cookie).body(()).unwrap().into_parts();
        crate::passkeys::owner_from_cookie(&state.db, &parts).await.unwrap()
    }

    #[tokio::test]
    async fn the_owner_signs_in_and_a_back_channel_logout_signs_them_out() {
        let f = Issuer::start().await;
        let state = deck(&f, "sam").await;
        let res = sign_in(&state, &f, "p_sam", "sam", true).await;
        assert_eq!(res.status(), StatusCode::OK);
        let cookie = session_cookie(&res).expect("a session cookie");
        let page = String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
        assert!(page.contains("url=/work"), "moves on to where the person was going: {page}");
        let who = signed_in(&state, &cookie).await.expect("signed in");
        assert_eq!(who, crate::passkeys::SignedIn { passkey_id: None, device: "sam (Fleetlayer sign-in)".into() });

        let logout = |token: String| {
            Request::post("/auth/backchannel-logout")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(format!("logout_token={token}")))
                .unwrap()
        };
        // Someone else's sign-in session ending leaves this one.
        assert_eq!(call(&state, logout(f.logout_token("p_sam", "sid-other"))).await.status(), StatusCode::OK);
        assert!(signed_in(&state, &cookie).await.is_some());
        assert_eq!(call(&state, logout(f.logout_token("p_sam", "sid-1"))).await.status(), StatusCode::OK);
        assert!(signed_in(&state, &cookie).await.is_none(), "signed out");
        // A token that isn't the issuer's is refused.
        let forged = logout("eyJhbGciOiJub25lIn0.e30.".into());
        assert_eq!(call(&state, forged).await.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn anyone_but_the_owner_is_refused_and_a_sign_in_needs_its_own_browsers_state() {
        let f = Issuer::start().await;
        let state = deck(&f, "p_sam").await;
        let res = sign_in(&state, &f, "p_robin", "robin", true).await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        assert!(session_cookie(&res).is_none());
        assert_eq!(sign_in(&state, &f, "p_sam", "sam", true).await.status(), StatusCode::OK, "the owner by ID");
        let res = sign_in(&state, &f, "p_sam", "sam", false).await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "started in another browser");
        assert!(session_cookie(&res).is_none());
    }

    #[test]
    fn next_stays_on_the_deck() {
        assert_eq!(safe_next(Some("/work?x=1")), "/work?x=1");
        assert_eq!(safe_next(Some("//evil.example")), "/");
        assert_eq!(safe_next(Some("https://evil.example")), "/");
        assert_eq!(safe_next(Some("/\\evil.example")), "/");
        assert_eq!(safe_next(None), "/");
    }

    #[tokio::test]
    async fn without_fleetlayers_sign_in_the_routes_say_so() {
        let state = AppState::for_tests().await;
        let res = call(&state, Request::get("/auth/signin").body(Body::empty()).unwrap()).await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }
}
