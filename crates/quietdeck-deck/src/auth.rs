//! Who's calling: agents with a session token from their node, nodes with their node token, and the owner in a
//! browser (Tailscale identity now; the passkey sign-in joins it in the next step).
//!
//! Tokens:
//! - node tokens `qdn_…`, issued at enrolment, stored as SHA-256 hashes;
//! - session tokens `qds_<claims>.<signature>`, minted by the deck for a node's caller over the node's WebSocket,
//!   signed with HMAC-SHA256 under a key the deck makes for itself, valid for an hour;
//! - enrolment codes, one-time, valid for 15 minutes, stored hashed.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use hmac::{Hmac, KeyInit, Mac};
use quietdeck_types::SessionRef;
use quietdeck_types::api::Caller;
use quietdeck_types::node::TokenClaims;
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::db::{Db, now_ms};
use crate::error::AppError;
use crate::state::AppState;

pub const SESSION_TOKEN_TTL_SECONDS: i64 = 3600;
pub const ENROLMENT_CODE_TTL_MS: i64 = 15 * 60 * 1000;
/// The tab a caller with no herdr tab (a plain shell) is shown as.
pub const SHELL_TAB: &str = "shell";

pub fn sha256_hex(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    rand::rng().fill_bytes(&mut b);
    b
}

pub fn new_node_token() -> String {
    format!("qdn_{}", B64.encode(random_bytes::<32>()))
}

/// An enrolment code people can type: 16 characters from an alphabet with no look-alikes, in groups of four.
pub fn new_enrolment_code() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
    let bytes = random_bytes::<16>();
    let chars: Vec<char> = bytes.iter().map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char).collect();
    chars.chunks(4).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join("-")
}

/// Codes are compared without dashes, spaces or case.
pub fn normalise_code(code: &str) -> String {
    code.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).collect()
}

/// The deck's own signing key, made on first start and kept in the database.
pub async fn signing_key(db: &Db) -> anyhow::Result<Vec<u8>> {
    if let Some((key,)) = sqlx::query_as::<_, (Vec<u8>,)>("SELECT value FROM deck_secrets WHERE name = 'session_key'")
        .fetch_optional(db)
        .await?
    {
        return Ok(key);
    }
    let key = random_bytes::<32>().to_vec();
    sqlx::query("INSERT OR IGNORE INTO deck_secrets (name, value, created_at) VALUES ('session_key', ?, ?)")
        .bind(&key)
        .bind(now_ms())
        .execute(db)
        .await?;
    // Another start may have won the race: read back whatever is stored.
    let (key,) = sqlx::query_as::<_, (Vec<u8>,)>("SELECT value FROM deck_secrets WHERE name = 'session_key'")
        .fetch_one(db)
        .await?;
    Ok(key)
}

fn sign(key: &[u8], payload: &str) -> Vec<u8> {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(payload.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

pub fn mint_session_token(key: &[u8], claims: &TokenClaims) -> String {
    let payload = B64.encode(serde_json::to_vec(claims).expect("claims serialise"));
    let sig = B64.encode(sign(key, &payload));
    format!("qds_{payload}.{sig}")
}

pub fn verify_session_token(key: &[u8], token: &str, now_secs: i64) -> Result<TokenClaims, &'static str> {
    let rest = token.strip_prefix("qds_").ok_or("not a session token")?;
    let (payload, sig) = rest.split_once('.').ok_or("malformed session token")?;
    let sig = B64.decode(sig).map_err(|_| "malformed session token")?;
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(payload.as_bytes());
    mac.verify_slice(&sig).map_err(|_| "session token signature doesn't match")?;
    let claims: TokenClaims = serde_json::from_slice(&B64.decode(payload).map_err(|_| "malformed session token")?)
        .map_err(|_| "malformed session token")?;
    if claims.exp < now_secs {
        return Err("session token expired: ask the node for a new one");
    }
    Ok(claims)
}

pub fn caller_ref(claims: &TokenClaims) -> SessionRef {
    SessionRef {
        node: claims.node.clone(),
        tab: claims.tab.clone().unwrap_or_else(|| SHELL_TAB.to_string()),
        session_id: claims.session_id.clone(),
    }
}

/// Resolve the request's caller.
pub async fn resolve(state: &AppState, parts: &Parts) -> Result<Caller, AppError> {
    if let Some(value) = parts.headers.get(axum::http::header::AUTHORIZATION) {
        let value = value.to_str().map_err(|_| AppError::Unauthorized("bad Authorization header".into()))?;
        let token = value
            .strip_prefix("Bearer ")
            .ok_or_else(|| AppError::Unauthorized("Authorization must be a Bearer token".into()))?
            .trim();
        if token.starts_with("qds_") {
            let claims = verify_session_token(&state.signing_key, token, now_ms() / 1000)
                .map_err(|e| AppError::Unauthorized(e.into()))?;
            if !node_active(&state.db, &claims.node).await? {
                return Err(AppError::Unauthorized(format!("node {} isn't enrolled", claims.node)));
            }
            return Ok(Caller::Session { session: caller_ref(&claims) });
        }
        if token.starts_with("qdn_") {
            let node = node_for_token(&state.db, token)
                .await?
                .ok_or_else(|| AppError::Unauthorized("unknown node token".into()))?;
            return Ok(Caller::Node { node });
        }
        return Err(AppError::Unauthorized("unknown token type".into()));
    }
    let login = parts.headers.get("tailscale-user-login").and_then(|v| v.to_str().ok());
    match login {
        Some(login) if state.owner_logins.iter().any(|o| o.eq_ignore_ascii_case(login.trim())) => {
            Ok(Caller::Owner { login: login.trim().to_ascii_lowercase() })
        }
        Some(_) => Err(AppError::Forbidden("this deck belongs to someone else".into())),
        None if state.dev_trust_local && !state.owner_logins.is_empty() => {
            Ok(Caller::Owner { login: state.owner_logins[0].clone() })
        }
        None => Err(AppError::Unauthorized(
            "sign in: a session token from your node (quietdeck CLI), or the web app over the tailnet".into(),
        )),
    }
}

async fn node_active(db: &Db, node: &str) -> Result<bool, AppError> {
    let row: Option<(i64,)> = sqlx::query_as("SELECT id FROM nodes WHERE name = ? AND revoked_at IS NULL")
        .bind(node)
        .fetch_optional(db)
        .await?;
    Ok(row.is_some())
}

pub async fn node_for_token(db: &Db, token: &str) -> Result<Option<String>, AppError> {
    let row: Option<(String,)> = sqlx::query_as("SELECT name FROM nodes WHERE token_hash = ? AND revoked_at IS NULL")
        .bind(sha256_hex(token))
        .fetch_optional(db)
        .await?;
    Ok(row.map(|r| r.0))
}

/// Any authenticated caller.
pub struct Authed(pub Caller);

impl FromRequestParts<AppState> for Authed {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        Ok(Authed(resolve(state, parts).await?))
    }
}

/// An agent (or shell) on a node, with a session token: the caller is a session.
pub struct Agent(pub SessionRef);

impl FromRequestParts<AppState> for Agent {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        match resolve(state, parts).await? {
            Caller::Session { session } => Ok(Agent(session)),
            _ => Err(AppError::Forbidden("this needs a session token from your node (the quietdeck CLI)".into())),
        }
    }
}

/// The owner in a browser.
pub struct Owner(#[allow(dead_code)] pub String);

impl FromRequestParts<AppState> for Owner {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        match resolve(state, parts).await? {
            Caller::Owner { login } => Ok(Owner(login)),
            _ => Err(AppError::Forbidden("only the owner, in the web app, can do this".into())),
        }
    }
}

/// A node (its runner) with its node token.
#[expect(dead_code, reason = "the runner's endpoints (build step 5) take it")]
pub struct NodeCaller(pub String);

impl FromRequestParts<AppState> for NodeCaller {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        match resolve(state, parts).await? {
            Caller::Node { node } => Ok(NodeCaller(node)),
            _ => Err(AppError::Forbidden("this needs a node token".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims(exp: i64) -> TokenClaims {
        TokenClaims {
            node: "laptop".into(),
            session_id: Some("s1".into()),
            tab: Some("billing".into()),
            pane: None,
            exp,
        }
    }

    #[test]
    fn session_tokens_verify_and_expire() {
        let key = b"0123456789abcdef0123456789abcdef";
        let t = mint_session_token(key, &claims(1000));
        assert_eq!(verify_session_token(key, &t, 999).unwrap(), claims(1000));
        assert!(verify_session_token(key, &t, 1001).unwrap_err().contains("expired"));
        assert!(verify_session_token(b"another key", &t, 999).is_err());
    }

    #[test]
    fn a_tampered_token_is_refused() {
        let key = b"k";
        let t = mint_session_token(key, &claims(1000));
        let forged = mint_session_token(b"other", &TokenClaims { node: "server".into(), ..claims(1000) });
        let (_, sig) = t.split_once('.').unwrap();
        let (payload, _) = forged.split_once('.').unwrap();
        assert!(verify_session_token(key, &format!("{payload}.{sig}"), 0).is_err());
    }

    #[test]
    fn codes_are_typeable() {
        let c = new_enrolment_code();
        assert_eq!(c.len(), 19);
        assert_eq!(normalise_code(&c.to_lowercase().replace('-', " ")), c.replace('-', ""));
    }
}
