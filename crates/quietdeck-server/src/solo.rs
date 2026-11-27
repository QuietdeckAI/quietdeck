//! Quietdeck solo's deck: on the person's own machine, at http://127.0.0.1:<port>, with no passkeys. Being on
//! localhost isn't enough on its own, since any page the person visits can send requests to 127.0.0.1 (form posts and
//! no-cors fetches carry no credentials, yet would approve or archive), and DNS rebinding can read the answers. So
//! every request goes through [`guard`]:
//!
//! - The Host must be this deck's own loopback address and port, which stops DNS rebinding.
//! - A request with an `Authorization` header (the CLI and the node) goes on to the usual token checks, which never
//!   trust a request by address and never make a token the owner. A page can't send that header to another origin
//!   without a CORS preflight, and the deck answers no preflight with an `Access-Control-Allow-*` header.
//! - Any other request (a browser's) needs the solo session cookie, which only a one-time login link sets:
//!   `quietdeck-solo open` writes the link's code to the data directory ([`mint_login_code`]), and the first visit
//!   uses it up. Nothing is trusted by address.
//! - A browser request that changes something must also come from the deck's own pages (Origin, or Referer when a
//!   browser sends no Origin).
//!
//! The codes and sessions are files in `<data dir>/solo` (mode 700), so the deck's database needs no migration.
//! `/api/solo/health` answers how the runner and its agent are doing, 503 when they're stale, for a local checker and
//! the Inbox.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use serde::{Deserialize, Serialize};

use crate::auth::{random_bytes, sha256_hex};
use crate::state::AppState;

pub const COOKIE_NAME: &str = "qd_solo";
/// How long a login link works, once.
const CODE_TTL_SECS: i64 = 5 * 60;
/// A session not used for this long ends.
const IDLE_SECS: i64 = 30 * 24 * 3600;
/// The mail watch counts as stale once its last good check is this old.
const MAIL_STALE_SECS: i64 = 10 * 60;
/// After start, how long the runner has to report before its silence is a problem.
const FIRST_REPORT_SECS: i64 = 3 * 60;

/// The listen address must be loopback: the port it binds, or why it can't.
pub fn loopback_port(listen: &str) -> anyhow::Result<u16> {
    let addr: std::net::SocketAddr = listen.parse().map_err(|e| {
        anyhow::anyhow!("solo listens on an IP address and port, like 127.0.0.1:7350 ({listen:?}: {e})")
    })?;
    anyhow::ensure!(
        addr.ip().is_loopback(),
        "solo's deck has no passkeys, so it listens only on this machine (127.0.0.1), never on {listen}"
    );
    Ok(addr.port())
}

/// The solo gate's state: the deck's port, and the sessions the login links made.
pub struct LocalOnly {
    port: u16,
    dir: PathBuf,
    sessions: Mutex<HashMap<String, i64>>,
}

#[derive(Default, Serialize, Deserialize)]
struct SessionsFile {
    /// Token hash → last used (Unix seconds).
    sessions: HashMap<String, i64>,
}

fn now() -> i64 {
    jiff::Timestamp::now().as_second()
}

fn codes_dir(dir: &Path) -> PathBuf {
    dir.join("login-codes")
}

fn private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// A one-time login code, good for [`CODE_TTL_SECS`], for `http://127.0.0.1:<port>/solo/login?code=<code>`.
/// Written by `quietdeck-solo open`, which may run beside the deck: each code is a file of its own, named by its hash.
pub fn mint_login_code(dir: &Path) -> anyhow::Result<String> {
    let codes = codes_dir(dir);
    private_dir(dir)?;
    private_dir(&codes)?;
    let code = B64.encode(random_bytes::<24>());
    std::fs::write(codes.join(sha256_hex(&code)), (now() + CODE_TTL_SECS).to_string())?;
    Ok(code)
}

impl LocalOnly {
    /// `dir` holds the codes and sessions (`<data dir>/solo`).
    pub fn new(port: u16, dir: PathBuf) -> anyhow::Result<LocalOnly> {
        private_dir(&dir)?;
        let file: SessionsFile = std::fs::read(dir.join("sessions.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Ok(LocalOnly { port, dir, sessions: Mutex::new(file.sessions) })
    }

    fn save(&self, sessions: &HashMap<String, i64>) {
        let path = self.dir.join("sessions.json");
        let tmp = path.with_extension("tmp");
        let body = serde_json::to_vec(&SessionsFile { sessions: sessions.clone() }).expect("serialises");
        if let Err(e) = std::fs::write(&tmp, body).and_then(|_| std::fs::rename(&tmp, &path)) {
            tracing::warn!(error = %e, "saving solo's sign-in sessions");
        }
    }

    /// Use up a login code: true when it existed and hadn't expired. Removing the file is what uses it, so two
    /// visits can't both succeed.
    fn take_code(&self, code: &str) -> bool {
        if code.is_empty()
            || code.len() > 100
            || !code.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return false;
        }
        let path = codes_dir(&self.dir).join(sha256_hex(code));
        let Ok(expires) = std::fs::read_to_string(&path) else { return false };
        if std::fs::remove_file(&path).is_err() {
            return false;
        }
        expires.trim().parse::<i64>().is_ok_and(|e| e > now())
    }

    fn new_session(&self) -> String {
        let token = B64.encode(random_bytes::<32>());
        let mut s = self.sessions.lock().expect("solo sessions");
        let t = now();
        s.retain(|_, last| t - *last < IDLE_SECS);
        s.insert(sha256_hex(&token), t);
        self.save(&s);
        token
    }

    fn session_ok(&self, headers: &HeaderMap) -> bool {
        let Some(token) = cookie(headers, COOKIE_NAME) else { return false };
        let hash = sha256_hex(&token);
        let mut s = self.sessions.lock().expect("solo sessions");
        let t = now();
        match s.get_mut(&hash) {
            Some(last) if t - *last < IDLE_SECS => {
                if t - *last > 3600 {
                    *last = t;
                    let snapshot = s.clone();
                    self.save(&snapshot);
                }
                true
            }
            Some(_) => {
                s.remove(&hash);
                false
            }
            None => false,
        }
    }

    /// This deck's own `host:port` forms.
    fn own_host(&self, host: &str) -> bool {
        let p = self.port;
        [format!("127.0.0.1:{p}"), format!("localhost:{p}"), format!("[::1]:{p}")].iter().any(|h| h == host)
    }
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(';')).find_map(|p| {
        let (n, v) = p.trim().split_once('=')?;
        (n == name && !v.is_empty()).then(|| v.to_string())
    })
}

fn refuse(status: StatusCode, why: &str) -> Response {
    (status, why.to_string()).into_response()
}

/// Paths a browser may reach without signing in: the health checks, and the login link itself.
fn open_path(path: &str) -> bool {
    matches!(path, "/healthz" | "/api/solo/health" | "/solo/login")
}

/// Every request to solo's deck passes here first.
pub async fn guard(State(lo): State<Arc<LocalOnly>>, req: Request, next: Next) -> Response {
    // HTTP/1.1 sends a Host header; HTTP/2 puts the host in the request's authority instead.
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .or_else(|| req.uri().authority().map(|a| a.to_string()))
        .unwrap_or_default();
    if !lo.own_host(&host) {
        return refuse(StatusCode::MISDIRECTED_REQUEST, "this deck answers only at its own address on this machine");
    }
    // The CLI and the node: their tokens are checked as usual further in.
    if req.headers().contains_key(header::AUTHORIZATION) {
        return next.run(req).await;
    }
    let changes = !matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    if changes {
        let own = format!("http://{host}");
        let origin = req.headers().get(header::ORIGIN).and_then(|v| v.to_str().ok());
        let referer = req.headers().get(header::REFERER).and_then(|v| v.to_str().ok());
        let from_own_pages = match (origin, referer) {
            (Some(o), _) => o.trim_end_matches('/') == own,
            (None, Some(r)) => r == own || r.starts_with(&format!("{own}/")),
            (None, None) => false,
        };
        if !from_own_pages {
            return refuse(StatusCode::FORBIDDEN, "this request didn't come from the deck's own pages");
        }
    }
    let path = req.uri().path().to_string();
    if path == "/solo/login" {
        let code =
            req.uri().query().and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("code=")).map(str::to_string));
        if !code.is_some_and(|c| lo.take_code(&c)) {
            return refuse(
                StatusCode::UNAUTHORIZED,
                "this login link has been used or has expired: run `quietdeck-solo open` for a new one",
            );
        }
        let token = lo.new_session();
        let mut res = Redirect::to("/").into_response();
        let cookie = format!("{COOKIE_NAME}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age=34560000");
        res.headers_mut().insert(header::SET_COOKIE, HeaderValue::from_str(&cookie).expect("ASCII"));
        return res;
    }
    if !open_path(&path) && !lo.session_ok(req.headers()) {
        return refuse(StatusCode::UNAUTHORIZED, "sign in on this machine: run `quietdeck-solo open`");
    }
    next.run(req).await
}

/// `/api/solo/health`.
pub fn router() -> axum::Router<AppState> {
    axum::Router::new().route("/api/solo/health", get(health))
}

#[derive(Debug, Serialize)]
pub struct SoloHealth {
    pub ok: bool,
    /// What's wrong, in words, for the Inbox's warning and the checker's log.
    pub problems: Vec<String>,
    pub runner: Option<quietdeck_types::node::RunnerHealth>,
    /// When the deck last heard from the runner.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reported_at: Option<jiff::Timestamp>,
}

/// What's wrong with the runner's last report, given when the deck started.
pub fn assess(
    report: Option<&(quietdeck_types::node::RunnerHealth, jiff::Timestamp)>,
    started: jiff::Timestamp,
    now: jiff::Timestamp,
) -> SoloHealth {
    let age = |t: jiff::Timestamp| now.as_second() - t.as_second();
    let mut problems = vec![];
    match report {
        None if age(started) > FIRST_REPORT_SECS => {
            problems.push("the runner hasn't reported since the deck started".to_string())
        }
        None => {}
        Some((h, at)) => {
            if age(*at) > MAIL_STALE_SECS {
                problems.push(format!("the runner last reported {} minutes ago", age(*at) / 60));
            }
            match h.mail_ok_at {
                Some(t) if age(t) > MAIL_STALE_SECS => {
                    problems.push(format!("mail was last read {} minutes ago", age(t) / 60))
                }
                None if h.mail_checked_at.is_some() => problems.push("mail hasn't been read yet".to_string()),
                _ => {}
            }
            if let Some(failed) = h.agent_failed_at {
                let why = h.agent_error.as_deref().unwrap_or("no reason given");
                problems.push(format!("the agent's last turn failed {} minutes ago: {why}", age(failed) / 60));
            }
        }
    }
    SoloHealth {
        ok: problems.is_empty(),
        problems,
        runner: report.map(|(h, _)| h.clone()),
        reported_at: report.map(|(_, at)| *at),
    }
}

async fn health(State(state): State<AppState>) -> Response {
    let report = state.runner_health.lock().expect("runner health").values().max_by_key(|(_, at)| *at).cloned();
    let h = assess(report.as_ref(), *crate::state::STARTED_AT, jiff::Timestamp::now());
    let status = if h.ok { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };
    (status, Json(h)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use tower::ServiceExt;

    const PORT: u16 = 7350;

    fn app(lo: Arc<LocalOnly>) -> axum::Router {
        axum::Router::new()
            .route("/", get(|| async { "inbox" }))
            .route("/api/cards", get(|| async { "cards" }).post(|| async { "posted" }))
            .route("/healthz", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(lo, guard))
    }

    fn req(method: &str, path: &str, headers: &[(&str, &str)]) -> Request {
        let mut b = Request::builder().method(method).uri(path).header("host", format!("127.0.0.1:{PORT}"));
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        b.body(Body::empty()).unwrap()
    }

    async fn send(app: &axum::Router, r: Request) -> Response {
        app.clone().oneshot(r).await.unwrap()
    }

    fn setup() -> (tempfile::TempDir, Arc<LocalOnly>, axum::Router) {
        let dir = tempfile::tempdir().unwrap();
        let lo = Arc::new(LocalOnly::new(PORT, dir.path().join("solo")).unwrap());
        let a = app(lo.clone());
        (dir, lo, a)
    }

    async fn signed_in(dir: &Path, app: &axum::Router) -> String {
        let code = mint_login_code(&dir.join("solo")).unwrap();
        let res = send(app, req("GET", &format!("/solo/login?code={code}"), &[])).await;
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        let c = res.headers().get(header::SET_COOKIE).unwrap().to_str().unwrap().to_string();
        assert!(c.contains("HttpOnly") && c.contains("SameSite=Strict"), "{c}");
        c.split(';').next().unwrap().to_string()
    }

    #[test]
    fn only_loopback_listens() {
        assert_eq!(loopback_port("127.0.0.1:7350").unwrap(), 7350);
        assert_eq!(loopback_port("[::1]:7350").unwrap(), 7350);
        assert!(loopback_port("0.0.0.0:7350").unwrap_err().to_string().contains("only on this machine"));
        assert!(loopback_port("192.168.1.5:7350").is_err());
        assert!(loopback_port("localhost:7350").is_err(), "a name could resolve anywhere");
    }

    #[tokio::test]
    async fn a_foreign_host_is_refused_even_with_a_session() {
        let (dir, _lo, app) = setup();
        let cookie = signed_in(dir.path(), &app).await;
        let mut r = req("GET", "/", &[("cookie", &cookie)]);
        r.headers_mut().insert("host", HeaderValue::from_static("evil.example:7350"));
        assert_eq!(send(&app, r).await.status(), StatusCode::MISDIRECTED_REQUEST, "DNS rebinding");
        let mut r = req("GET", "/", &[("cookie", &cookie)]);
        r.headers_mut().insert("host", HeaderValue::from_static("127.0.0.1:9999"));
        assert_eq!(send(&app, r).await.status(), StatusCode::MISDIRECTED_REQUEST, "another port");
        let mut r = req("GET", "/", &[]);
        r.headers_mut().remove("host");
        assert_eq!(send(&app, r).await.status(), StatusCode::MISDIRECTED_REQUEST, "no Host at all");
        let mut r = req("GET", "/", &[("cookie", &cookie)]);
        r.headers_mut().insert("host", HeaderValue::from_static("localhost:7350"));
        assert_eq!(send(&app, r).await.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn a_browser_without_a_session_is_refused_and_nothing_is_trusted_by_address() {
        let (_dir, _lo, app) = setup();
        assert_eq!(send(&app, req("GET", "/", &[])).await.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(send(&app, req("GET", "/api/cards", &[])).await.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            send(&app, req("GET", "/", &[("cookie", "qd_solo=made-up")])).await.status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(send(&app, req("GET", "/healthz", &[])).await.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn a_login_link_works_once_and_not_after_it_expires() {
        let (dir, lo, app) = setup();
        let code = mint_login_code(&dir.path().join("solo")).unwrap();
        let first = send(&app, req("GET", &format!("/solo/login?code={code}"), &[])).await;
        assert_eq!(first.status(), StatusCode::SEE_OTHER);
        let again = send(&app, req("GET", &format!("/solo/login?code={code}"), &[])).await;
        assert_eq!(again.status(), StatusCode::UNAUTHORIZED, "used up");
        assert_eq!(send(&app, req("GET", "/solo/login?code=guess", &[])).await.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(send(&app, req("GET", "/solo/login", &[])).await.status(), StatusCode::UNAUTHORIZED);

        let old = mint_login_code(&dir.path().join("solo")).unwrap();
        std::fs::write(codes_dir(&lo.dir).join(sha256_hex(&old)), (now() - 1).to_string()).unwrap();
        let expired = send(&app, req("GET", &format!("/solo/login?code={old}"), &[])).await;
        assert_eq!(expired.status(), StatusCode::UNAUTHORIZED);

        // The session outlives a restart of the deck.
        let cookie = signed_in(dir.path(), &app).await;
        let restarted = app_from(&dir);
        assert_eq!(send(&restarted, req("GET", "/", &[("cookie", &cookie)])).await.status(), StatusCode::OK);
    }

    fn app_from(dir: &tempfile::TempDir) -> axum::Router {
        app(Arc::new(LocalOnly::new(PORT, dir.path().join("solo")).unwrap()))
    }

    #[tokio::test]
    async fn changes_need_the_decks_own_origin() {
        let (dir, _lo, app) = setup();
        let cookie = signed_in(dir.path(), &app).await;
        let own = format!("http://127.0.0.1:{PORT}");
        let post = |h: &[(&str, &str)]| {
            let mut all = vec![("cookie", cookie.as_str())];
            all.extend_from_slice(h);
            req("POST", "/api/cards", &all)
        };
        assert_eq!(send(&app, post(&[("origin", &own)])).await.status(), StatusCode::OK);
        assert_eq!(send(&app, post(&[("origin", "https://evil.example")])).await.status(), StatusCode::FORBIDDEN);
        assert_eq!(send(&app, post(&[("origin", "null")])).await.status(), StatusCode::FORBIDDEN);
        assert_eq!(send(&app, post(&[])).await.status(), StatusCode::FORBIDDEN, "no Origin and no Referer");
        let referer = format!("{own}/inbox");
        assert_eq!(send(&app, post(&[("referer", &referer)])).await.status(), StatusCode::OK);
        let lookalike = format!("{own}.evil.example/");
        assert_eq!(send(&app, post(&[("referer", &lookalike)])).await.status(), StatusCode::FORBIDDEN);
        // The login link is a GET, but a cross-site POST to it is refused like any other.
        let login = req("POST", "/solo/login?code=x", &[("origin", "https://evil.example")]);
        assert_eq!(send(&app, login).await.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn tokens_go_on_to_their_own_checks_and_preflights_get_no_cors() {
        let (_dir, _lo, app) = setup();
        // A Bearer request isn't the gate's to judge (auth.rs checks the token); without a session it still passes.
        let r = req("POST", "/api/cards", &[("authorization", "Bearer qds_x")]);
        assert_eq!(send(&app, r).await.status(), StatusCode::OK);
        // A page on another origin would need a preflight to send Authorization: it gets no CORS headers.
        let pre = req(
            "OPTIONS",
            "/api/cards",
            &[
                ("origin", "https://evil.example"),
                ("access-control-request-method", "POST"),
                ("access-control-request-headers", "authorization"),
            ],
        );
        let res = send(&app, pre).await;
        assert!(
            !res.headers().keys().any(|k| k.as_str().starts_with("access-control-")),
            "no Access-Control-Allow-* on a preflight: {:?}",
            res.headers()
        );
    }

    #[test]
    fn health_says_what_is_stale() {
        use quietdeck_types::node::RunnerHealth;
        let now = jiff::Timestamp::now();
        let ago = |s: i64| jiff::Timestamp::from_second(now.as_second() - s).unwrap();
        assert!(assess(None, ago(30), now).ok, "the runner has a moment to report after start");
        assert!(!assess(None, ago(600), now).ok);
        let good = RunnerHealth { mail_ok_at: Some(ago(20)), mail_checked_at: Some(ago(20)), ..Default::default() };
        assert!(assess(Some(&(good.clone(), ago(20))), ago(3600), now).ok);
        let stale = RunnerHealth { mail_ok_at: Some(ago(1200)), mail_checked_at: Some(ago(20)), ..good.clone() };
        let h = assess(Some(&(stale, ago(20))), ago(3600), now);
        assert_eq!(h.problems, vec!["mail was last read 20 minutes ago".to_string()]);
        let failing =
            RunnerHealth { agent_failed_at: Some(ago(120)), agent_error: Some("claude exited".into()), ..good.clone() };
        let h = assess(Some(&(failing, ago(20))), ago(3600), now);
        assert_eq!(h.problems, vec!["the agent's last turn failed 2 minutes ago: claude exited".to_string()]);
        assert!(!assess(Some(&(good, ago(1800))), ago(3600), now).ok, "a runner that went quiet");
    }
}
