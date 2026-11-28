//! Passkey sign-in for the owner's browsers (webauthn-rs, used unmodified). Nothing a browser can reach works without
//! it, except the sign-in screen itself.
//!
//! - The first passkey, and recovery when every passkey is lost, take a one-time setup code from the deck's
//!   container (`quietdeck-server setup-code`): access to the container is what proves it's the owner.
//! - Signed in, Settings adds a passkey per device.
//! - A sign-in makes a browser session: a random token in an HttpOnly, SameSite=Strict cookie, stored hashed, good
//!   while it's used at least once every 30 days.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use aide::axum::ApiRouter;
use aide::axum::routing::{get, patch, post};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::header::{COOKIE, SET_COOKIE, USER_AGENT};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use quietdeck_types::api::{AuthState, Ceremony, CeremonyFinish, PasskeyInfo, PasskeyRename, RegisterStart};
use sha2::{Digest, Sha256};
use webauthn_rs::prelude::{
    Passkey, PasskeyAuthentication, PasskeyRegistration, PublicKeyCredential, RegisterPublicKeyCredential, Url, Uuid,
    Webauthn, WebauthnBuilder,
};

use crate::auth::{Owner, normalise_code, random_bytes, sha256_hex};
use crate::db::{Db, now_ms, ts, ts_opt};
use crate::error::{AppError, AppResult, bad};
use crate::openapi::Id;
use crate::state::AppState;

pub const COOKIE_NAME: &str = "qd_auth";
/// A browser session ends after this long unused.
const IDLE_MS: i64 = 30 * 24 * 3600 * 1000;
/// `last_seen_at` is written at most this often.
const TOUCH_MS: i64 = 3600 * 1000;
const CEREMONY_TTL: Duration = Duration::from_secs(300);
pub const SETUP_CODE_TTL_MS: i64 = 15 * 60 * 1000;

pub fn webauthn(public_url: &str) -> anyhow::Result<Webauthn> {
    let origin = Url::parse(public_url).map_err(|e| anyhow::anyhow!("QUIETDECK_PUBLIC_URL {public_url:?}: {e}"))?;
    let rp_id = origin
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("QUIETDECK_PUBLIC_URL {public_url:?} has no host"))?
        .to_string();
    if rp_id.trim_matches(['[', ']']).parse::<std::net::IpAddr>().is_ok() {
        anyhow::bail!(
            "QUIETDECK_PUBLIC_URL {public_url:?} is an IP address, and passkeys need a domain name: use the name you \
             open the deck at (http://localhost:<port> works on the deck's own machine)"
        );
    }
    let built = WebauthnBuilder::new(&rp_id, &origin).and_then(|b| b.rp_name("Quietdeck").build());
    built.map_err(|e| anyhow::anyhow!("passkeys for QUIETDECK_PUBLIC_URL {public_url:?}: {e}"))
}

enum Pending {
    Register { state: PasskeyRegistration, device_name: String, setup_code_hash: Option<String> },
    SignIn { state: PasskeyAuthentication },
}

/// Ceremonies in progress, for five minutes each. Memory is enough: a restart only means trying again.
#[derive(Default)]
pub struct Ceremonies(Mutex<HashMap<String, (Instant, Pending)>>);

impl Ceremonies {
    fn put(&self, pending: Pending) -> String {
        let id = B64.encode(random_bytes::<16>());
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, (at, _)| at.elapsed() < CEREMONY_TTL);
        map.insert(id.clone(), (Instant::now(), pending));
        id
    }

    fn take(&self, id: &str) -> Result<Pending, AppError> {
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        match map.remove(id) {
            Some((at, p)) if at.elapsed() < CEREMONY_TTL => Ok(p),
            _ => Err(AppError::BadRequest("that sign-in attempt expired: start again".into())),
        }
    }
}

/// Sign-in routes, without the `/api` prefix (`crate::openapi` mounts them with the rest of the API).
pub fn router() -> ApiRouter<AppState> {
    ApiRouter::new()
        .api_route("/auth/state", get(auth_state))
        .api_route("/auth/register/start", post(register_start))
        .api_route("/auth/register/finish", post(register_finish))
        .api_route("/auth/login/start", post(login_start))
        .api_route("/auth/login/finish", post(login_finish))
        .api_route("/auth/logout", post(logout))
        .api_route("/passkeys", get(list))
        .api_route("/passkeys/{id}", patch(rename).delete(remove))
}

/// The cookie's value, when the request has one.
fn cookie_token(headers: &HeaderMap) -> Option<String> {
    headers.get_all(COOKIE).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(';')).find_map(|pair| {
        let (name, value) = pair.trim().split_once('=')?;
        (name == COOKIE_NAME && !value.is_empty()).then(|| value.to_string())
    })
}

/// A signed-in browser: its passkey, or none for a session from Fleetlayer's sign-in service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedIn {
    pub passkey_id: Option<i64>,
    /// How the Caller is described: `<device> (passkey <id>)`, or `<handle> (Fleetlayer sign-in)`.
    pub device: String,
}

/// A browser session's row: last seen, its passkey and that passkey's name, or the sign-in's person and handle.
type SessionRow = (i64, Option<i64>, Option<String>, Option<String>, Option<String>);

/// The signed-in browser, if the request's cookie is a live session.
pub async fn owner_from_cookie(db: &Db, parts: &Parts) -> Result<Option<SignedIn>, AppError> {
    let Some(token) = cookie_token(&parts.headers) else { return Ok(None) };
    let hash = sha256_hex(&token);
    let now = now_ms();
    // A passkey session ends with its passkey (ON DELETE CASCADE); a sign-in session has none.
    let row: Option<SessionRow> = sqlx::query_as(
        "SELECT s.last_seen_at, p.id, p.device_name, s.signin_sub, s.signin_handle FROM auth_sessions s \
         LEFT JOIN passkeys p ON p.id = s.passkey_id WHERE s.token_hash = ? \
         AND (p.id IS NOT NULL OR s.signin_sub IS NOT NULL)",
    )
    .bind(&hash)
    .fetch_optional(db)
    .await?;
    let Some((last_seen, passkey_id, device_name, signin_sub, signin_handle)) = row else { return Ok(None) };
    let signed_in = match passkey_id {
        Some(id) => SignedIn {
            passkey_id: Some(id),
            device: format!("{} (passkey {id})", device_name.as_deref().unwrap_or("passkey")),
        },
        None => SignedIn {
            passkey_id: None,
            device: format!(
                "{} (Fleetlayer sign-in)",
                signin_handle.or(signin_sub).unwrap_or_else(|| "the owner".into())
            ),
        },
    };
    if now - last_seen > IDLE_MS {
        sqlx::query("DELETE FROM auth_sessions WHERE token_hash = ?").bind(&hash).execute(db).await?;
        return Ok(None);
    }
    if now - last_seen > TOUCH_MS {
        sqlx::query("UPDATE auth_sessions SET last_seen_at = ? WHERE token_hash = ?")
            .bind(now)
            .bind(&hash)
            .execute(db)
            .await?;
    }
    Ok(Some(signed_in))
}

fn secure(state: &AppState) -> bool {
    state.public_url.starts_with("https://")
}

pub(crate) fn set_cookie(state: &AppState, token: &str) -> HeaderValue {
    // 400 days is the longest browsers keep a cookie; the session itself ends after 30 days unused.
    let attrs = if secure(state) { "; Secure" } else { "" };
    HeaderValue::from_str(&format!("{COOKIE_NAME}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age=34560000{attrs}"))
        .expect("the cookie is ASCII")
}

fn clear_cookie(state: &AppState) -> HeaderValue {
    let attrs = if secure(state) { "; Secure" } else { "" };
    HeaderValue::from_str(&format!("{COOKIE_NAME}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0{attrs}"))
        .expect("the cookie is ASCII")
}

/// The WebAuthn user handle: one owner per deck, so one handle, derived from the deck's own key.
fn user_handle(state: &AppState) -> Uuid {
    let digest = Sha256::new().chain_update(state.signing_key.as_slice()).chain_update(b"webauthn-user").finalize();
    let mut b = [0u8; 16];
    b.copy_from_slice(&digest[..16]);
    Uuid::from_bytes(b)
}

async fn load_passkeys(db: &Db) -> anyhow::Result<Vec<(i64, Passkey)>> {
    let rows: Vec<(i64, String)> = sqlx::query_as("SELECT id, passkey FROM passkeys ORDER BY id").fetch_all(db).await?;
    rows.into_iter().map(|(id, json)| Ok((id, serde_json::from_str(&json)?))).collect()
}

fn credential_id(p: &Passkey) -> String {
    B64.encode::<&[u8]>(p.cred_id().as_ref())
}

async fn auth_state(State(state): State<AppState>, parts: Parts) -> AppResult<Json<AuthState>> {
    let signed_in = owner_from_cookie(&state.db, &parts).await?.is_some() || state.dev_trust_local;
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM passkeys").fetch_one(&state.db).await?;
    let signin = state.signin.is_some();
    // With Fleetlayer's sign-in, no passkey is needed to get in, so no setup code is asked for first.
    Ok(Json(AuthState { signed_in, setup_needed: count.0 == 0 && !signin, signin }))
}

async fn setup_code_valid(db: &Db, code: &str) -> Result<String, AppError> {
    let hash = sha256_hex(&normalise_code(code));
    let ok: Option<(i64,)> = sqlx::query_as(
        "SELECT 1 FROM enrolment_codes WHERE code_hash = ? AND purpose = 'passkey' AND used_at IS NULL \
         AND expires_at > ?",
    )
    .bind(&hash)
    .bind(now_ms())
    .fetch_optional(db)
    .await?;
    match ok {
        Some(_) => Ok(hash),
        None => Err(AppError::Unauthorized(
            "that setup code is unknown, used or expired: run `quietdeck-server setup-code` in the deck's container"
                .into(),
        )),
    }
}

async fn register_start(
    State(state): State<AppState>,
    parts: Parts,
    Json(req): Json<RegisterStart>,
) -> AppResult<Json<Ceremony>> {
    garde::Validate::validate(&req).map_err(bad)?;
    // Signed in adds a device; otherwise only a setup code will do.
    let setup_code_hash = match &req.setup_code {
        Some(code) => Some(setup_code_valid(&state.db, code).await?),
        None => {
            if owner_from_cookie(&state.db, &parts).await?.is_none() {
                return Err(AppError::Unauthorized(
                    "sign in first, or use a setup code from `quietdeck-server setup-code`".into(),
                ));
            }
            None
        }
    };
    let existing = load_passkeys(&state.db).await?;
    let exclude = existing.iter().map(|(_, p)| p.cred_id().clone()).collect::<Vec<_>>();
    let (options, reg) = state
        .webauthn
        .start_passkey_registration(user_handle(&state), "owner", "Quietdeck owner", Some(exclude))
        .map_err(|e| AppError::Internal(anyhow::anyhow!("starting passkey registration: {e}")))?;
    let ceremony =
        state.ceremonies.put(Pending::Register { state: reg, device_name: req.device_name, setup_code_hash });
    Ok(Json(Ceremony { ceremony, options: serde_json::to_value(options).map_err(anyhow::Error::from)? }))
}

async fn register_finish(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CeremonyFinish>,
) -> AppResult<Response> {
    let Pending::Register { state: reg, device_name, setup_code_hash } = state.ceremonies.take(&req.ceremony)? else {
        return Err(AppError::BadRequest("that isn't a registration".into()));
    };
    let credential: RegisterPublicKeyCredential =
        serde_json::from_value(req.credential).map_err(|e| AppError::BadRequest(format!("the credential: {e}")))?;
    let passkey = state
        .webauthn
        .finish_passkey_registration(&credential, &reg)
        .map_err(|e| AppError::Unauthorized(format!("the passkey didn't verify: {e}")))?;
    let now = now_ms();
    let mut tx = state.db.begin().await?;
    if let Some(hash) = setup_code_hash {
        let used = sqlx::query(
            "UPDATE enrolment_codes SET used_at = ? WHERE code_hash = ? AND purpose = 'passkey' AND used_at IS NULL \
             AND expires_at > ?",
        )
        .bind(now)
        .bind(&hash)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        if used.rows_affected() != 1 {
            return Err(AppError::Unauthorized("that setup code was used or expired meanwhile".into()));
        }
    }
    let id = sqlx::query(
        "INSERT INTO passkeys (user_id, credential_id, passkey, device_name, created_at, last_used_at) \
         VALUES (1, ?, ?, ?, ?, ?)",
    )
    .bind(credential_id(&passkey))
    .bind(serde_json::to_string(&passkey).map_err(anyhow::Error::from)?)
    .bind(&device_name)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await?
    .last_insert_rowid();
    tx.commit().await?;
    tracing::info!(passkey = %device_name, "passkey registered");
    sign_in(&state, &headers, id).await
}

async fn sign_in(state: &AppState, headers: &HeaderMap, passkey_id: i64) -> AppResult<Response> {
    let token = B64.encode(random_bytes::<32>());
    let now = now_ms();
    let device: String =
        headers.get(USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("").chars().take(200).collect();
    sqlx::query(
        "INSERT INTO auth_sessions (token_hash, user_id, device, created_at, last_seen_at, verified_at, passkey_id) \
         VALUES (?, 1, ?, ?, ?, ?, ?)",
    )
    .bind(sha256_hex(&token))
    .bind(device)
    .bind(now)
    .bind(now)
    .bind(now)
    .bind(passkey_id)
    .execute(&state.db)
    .await?;
    let mut res =
        Json(AuthState { signed_in: true, setup_needed: false, signin: state.signin.is_some() }).into_response();
    res.headers_mut().insert(SET_COOKIE, set_cookie(state, &token));
    Ok(res)
}

async fn login_start(State(state): State<AppState>) -> AppResult<Json<Ceremony>> {
    let passkeys: Vec<Passkey> = load_passkeys(&state.db).await?.into_iter().map(|(_, p)| p).collect();
    if passkeys.is_empty() {
        return Err(AppError::Conflict("no passkey yet: register one with a setup code".into()));
    }
    let (options, auth) = state
        .webauthn
        .start_passkey_authentication(&passkeys)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("starting sign-in: {e}")))?;
    let ceremony = state.ceremonies.put(Pending::SignIn { state: auth });
    Ok(Json(Ceremony { ceremony, options: serde_json::to_value(options).map_err(anyhow::Error::from)? }))
}

async fn login_finish(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CeremonyFinish>,
) -> AppResult<Response> {
    let Pending::SignIn { state: auth } = state.ceremonies.take(&req.ceremony)? else {
        return Err(AppError::BadRequest("that isn't a sign-in".into()));
    };
    let credential: PublicKeyCredential =
        serde_json::from_value(req.credential).map_err(|e| AppError::BadRequest(format!("the credential: {e}")))?;
    let result = state
        .webauthn
        .finish_passkey_authentication(&credential, &auth)
        .map_err(|e| AppError::Unauthorized(format!("the passkey didn't verify: {e}")))?;
    let used = B64.encode::<&[u8]>(result.cred_id().as_ref());
    let mut found = None;
    for (id, mut passkey) in load_passkeys(&state.db).await? {
        if credential_id(&passkey) == used {
            let now = now_ms();
            if passkey.update_credential(&result) == Some(true) {
                sqlx::query("UPDATE passkeys SET passkey = ? WHERE id = ?")
                    .bind(serde_json::to_string(&passkey).map_err(anyhow::Error::from)?)
                    .bind(id)
                    .execute(&state.db)
                    .await?;
            }
            sqlx::query("UPDATE passkeys SET last_used_at = ? WHERE id = ?")
                .bind(now)
                .bind(id)
                .execute(&state.db)
                .await?;
            found = Some(id);
            break;
        }
    }
    let id = found.ok_or_else(|| AppError::Unauthorized("that passkey was removed".into()))?;
    sign_in(&state, &headers, id).await
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> AppResult<Response> {
    if let Some(token) = cookie_token(&headers) {
        sqlx::query("DELETE FROM auth_sessions WHERE token_hash = ?")
            .bind(sha256_hex(&token))
            .execute(&state.db)
            .await?;
    }
    let mut res =
        Json(AuthState { signed_in: false, setup_needed: false, signin: state.signin.is_some() }).into_response();
    res.headers_mut().insert(SET_COOKIE, clear_cookie(&state));
    Ok(res)
}

async fn list(State(state): State<AppState>, owner: Owner) -> AppResult<Json<Vec<PasskeyInfo>>> {
    let rows: Vec<(i64, Option<String>, i64, Option<i64>)> =
        sqlx::query_as("SELECT id, device_name, created_at, last_used_at FROM passkeys ORDER BY id")
            .fetch_all(&state.db)
            .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, name, created, used)| PasskeyInfo {
                id,
                device_name: name.unwrap_or_else(|| "passkey".into()),
                created_at: ts(created),
                last_used_at: ts_opt(used),
                this_device: owner.passkey_id == Some(id),
            })
            .collect(),
    ))
}

async fn rename(
    State(state): State<AppState>,
    _owner: Owner,
    Path(Id { id }): Path<Id>,
    Json(req): Json<PasskeyRename>,
) -> AppResult<()> {
    garde::Validate::validate(&req).map_err(bad)?;
    let res = sqlx::query("UPDATE passkeys SET device_name = ? WHERE id = ?")
        .bind(&req.device_name)
        .bind(id)
        .execute(&state.db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("no passkey {id}")));
    }
    Ok(())
}

/// Remove a passkey and sign out every browser that used it. The last one stays: without it, only a setup code
/// gets back in.
async fn remove(State(state): State<AppState>, _owner: Owner, Path(Id { id }): Path<Id>) -> AppResult<()> {
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM passkeys").fetch_one(&state.db).await?;
    if count.0 <= 1 {
        return Err(AppError::Conflict("that's the only passkey: add another before removing it".into()));
    }
    let mut tx = state.db.begin().await?;
    sqlx::query("DELETE FROM auth_sessions WHERE passkey_id = ?").bind(id).execute(&mut *tx).await?;
    let res = sqlx::query("DELETE FROM passkeys WHERE id = ?").bind(id).execute(&mut *tx).await?;
    if res.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("no passkey {id}")));
    }
    tx.commit().await?;
    Ok(())
}

/// A one-time code for registering a passkey without being signed in.
pub async fn create_setup_code(db: &Db) -> anyhow::Result<String> {
    let code = crate::auth::new_enrolment_code();
    sqlx::query("INSERT INTO enrolment_codes (code_hash, expires_at, purpose) VALUES (?, ?, 'passkey')")
        .bind(sha256_hex(&normalise_code(&code)))
        .bind(now_ms() + SETUP_CODE_TTL_MS)
        .execute(db)
        .await?;
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passkeys_need_a_domain_name() {
        assert!(webauthn("https://deck.example.com").is_ok());
        assert!(webauthn("http://localhost:18080").is_ok());
        let e = webauthn("http://127.0.0.1:18080").err().unwrap().to_string();
        assert!(e.contains("domain name"), "{e}");
        assert!(webauthn("http://[::1]:8080").is_err());
    }

    #[test]
    fn the_cookie_is_found_among_others() {
        let mut h = HeaderMap::new();
        h.insert(COOKIE, HeaderValue::from_static("a=1; qd_auth=tok; b=2"));
        assert_eq!(cookie_token(&h).as_deref(), Some("tok"));
        h.insert(COOKIE, HeaderValue::from_static("qd_auth="));
        assert_eq!(cookie_token(&h), None);
    }

    #[test]
    fn the_relying_party_is_the_public_host() {
        webauthn("https://quietdeck.example.com").unwrap();
        webauthn("http://localhost:18080").unwrap();
        assert!(webauthn("not a url").is_err());
    }

    #[tokio::test]
    async fn setup_codes_are_one_time_and_only_for_passkeys() {
        let db = crate::db::memory().await;
        let code = create_setup_code(&db).await.unwrap();
        let hash = setup_code_valid(&db, &code.to_lowercase()).await.unwrap();
        assert!(crate::store::nodes::enrol(&db, &code, "laptop").await.is_err(), "not a node enrolment code");
        sqlx::query("UPDATE enrolment_codes SET used_at = 1 WHERE code_hash = ?")
            .bind(hash)
            .execute(&db)
            .await
            .unwrap();
        assert!(setup_code_valid(&db, &code).await.is_err());
    }
}
