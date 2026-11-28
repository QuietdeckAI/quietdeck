//! Fleetlayer's access requests and admin windows on the deck, decided with an `approve`-scoped token
//! (`fleetlayer-control token add quietdeck-approve --scope approve --owner <person>`). That token reads and decides
//! approvals and closes windows, and nothing else. It lives with the deck (a secret no agent's Unix user can read),
//! never on a node. Opening a role window also needs the owner's passkey for that one request: the control plane
//! refuses it until the sign-in service's `/stepup` page reports a fresh confirmation, so the deck sends the browser
//! there and approves again when it comes back.

use std::sync::OnceLock;
use std::time::Duration;

use quietdeck_types::approvals::{FleetApproval, FleetApprovals, FleetDecide, FleetDecided, FleetWindow};
use serde::Deserialize;
use serde_json::json;

use crate::error::{AppError, AppResult};

/// Set once at start-up when the deck has an approve token.
pub static APPROVER: OnceLock<Approver> = OnceLock::new();

#[derive(Debug, Clone)]
pub struct Approver {
    http: reqwest::Client,
    base: String,
    token: String,
    /// The sign-in service, for the passkey page (`https://signin.example.com`).
    signin: Option<String>,
    /// Fleetlayer's place (tenant) slug, which the passkey page names.
    tenant: String,
    /// Where the passkey page sends the browser back to: the deck's own address. The web app takes the request's ID
    /// from there to wherever the Work document's tab is (its path follows the tab's name, which can change).
    return_to: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawApproval {
    id: String,
    from: String,
    verb: String,
    to: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    created_at: i64,
    #[serde(default)]
    expires_at: Option<i64>,
    #[serde(default)]
    line: Option<String>,
    #[serde(default)]
    detail: Option<String>,
    #[serde(default)]
    once: bool,
}

#[derive(Deserialize)]
struct RawApprovals {
    approvals: Vec<RawApproval>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawWindow {
    id: String,
    role: String,
    #[serde(default)]
    requested_by: Option<String>,
    #[serde(default)]
    opened_at: Option<i64>,
    #[serde(default)]
    ends_at: Option<i64>,
    #[serde(default)]
    closed_at: Option<i64>,
    #[serde(default)]
    problem: Option<String>,
}

#[derive(Deserialize)]
struct RawWindows {
    #[serde(default)]
    windows: Vec<RawWindow>,
}

/// A request to open a role's window, which needs the passkey (Fleetlayer's own test: verb `activate` on `window:*`).
pub fn is_window(verb: &str, to: &str) -> bool {
    verb == "activate" && to.starts_with("window:")
}

fn approvals_of(raw: RawApprovals) -> Vec<FleetApproval> {
    raw.approvals
        .into_iter()
        .filter(|a| a.status.as_deref().is_none_or(|s| s == "pending"))
        .map(|a| FleetApproval {
            window: is_window(&a.verb, &a.to),
            id: a.id,
            from: a.from,
            verb: a.verb,
            to: a.to,
            created_at: a.created_at,
            expires_at: a.expires_at,
            line: a.line.filter(|l| !l.trim().is_empty()),
            detail: a.detail,
            once: a.once,
        })
        .collect()
}

/// The windows still open (`closedAt` unset), newest first as Fleetlayer lists them.
fn open_windows(raw: RawWindows) -> Vec<FleetWindow> {
    raw.windows
        .into_iter()
        .filter(|w| w.closed_at.is_none())
        .map(|w| FleetWindow {
            id: w.id,
            role: w.role,
            requested_by: w.requested_by,
            opened_at: w.opened_at,
            ends_at: w.ends_at,
            problem: w.problem,
        })
        .collect()
}

/// Percent-encoding for a query value or a path segment: everything but letters, digits and `-_.~`.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

impl Approver {
    pub fn new(base: &str, token: &str, signin: Option<&str>, tenant: &str, public_url: &str) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(20))
            .build()?;
        Ok(Approver {
            http,
            base: base.trim_end_matches('/').to_string(),
            token: token.to_string(),
            signin: signin.map(|s| s.trim_end_matches('/').to_string()),
            tenant: tenant.to_string(),
            return_to: format!("{}/", public_url.trim_end_matches('/')),
        })
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let res = self
            .http
            .get(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| format!("Fleetlayer didn't answer: {e}"))?;
        let status = res.status();
        if !status.is_success() {
            return Err(format!("GET {path} answered {status}: {}", res.text().await.unwrap_or_default()));
        }
        res.json().await.map_err(|e| format!("GET {path}: {e}"))
    }

    pub async fn list(&self) -> FleetApprovals {
        let (a, w) = tokio::join!(self.get::<RawApprovals>("/v1/approvals"), self.get::<RawWindows>("/v1/windows"));
        let error = [a.as_ref().err(), w.as_ref().err()].into_iter().flatten().cloned().collect::<Vec<_>>();
        FleetApprovals {
            configured: true,
            approvals: a.map(approvals_of).unwrap_or_default(),
            windows: w.map(open_windows).unwrap_or_default(),
            error: (!error.is_empty()).then(|| error.join("; ")),
        }
    }

    /// The sign-in service's page that confirms this one request with the owner's passkey, and comes back to the
    /// deck with the request's ID (`/?fleet_approve=<id>`), so the Work tab approves it again.
    pub fn stepup_url(&self, id: &str) -> Option<String> {
        let signin = self.signin.as_ref()?;
        let back = format!("{}?fleet_approve={}", self.return_to, encode(id));
        Some(format!(
            "{signin}/stepup?tenant={}&approval={}&return={}",
            encode(&self.tenant),
            encode(id),
            encode(&back)
        ))
    }

    pub async fn decide(&self, id: &str, d: &FleetDecide) -> AppResult<FleetDecided> {
        let path = format!("/v1/approvals/{}", encode(id));
        let mut body = json!({"approve": d.approve});
        if let Some(h) = &d.hold {
            body["for"] = json!(h);
        }
        let res = self
            .http
            .post(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::Unavailable(format!("Fleetlayer didn't answer: {e}")))?;
        let status = res.status();
        if status.is_success() {
            return Ok(FleetDecided { decided: true, stepup_url: None });
        }
        let text = res.text().await.unwrap_or_default();
        // A window waits for the passkey: send the browser to confirm it, then approve again.
        if status == reqwest::StatusCode::FORBIDDEN && d.approve && text.contains("passkey") {
            return match self.stepup_url(id) {
                Some(url) => Ok(FleetDecided { decided: false, stepup_url: Some(url) }),
                None => Err(AppError::BadRequest(
                    "opening a window needs your passkey, but the deck doesn't know the sign-in service \
                     (QUIETDECK_FLEETLAYER_SIGNIN_URL)"
                        .into(),
                )),
            };
        }
        Err(match status.as_u16() {
            404 => AppError::NotFound(format!("Fleetlayer has no pending request {id}: decided already, or lapsed")),
            400 => AppError::BadRequest(text),
            _ => AppError::Unavailable(format!("Fleetlayer answered {status}: {text}")),
        })
    }

    pub async fn close(&self, role: &str) -> AppResult<bool> {
        let path = format!("/v1/windows/{}", encode(role));
        let res = self
            .http
            .delete(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| AppError::Unavailable(format!("Fleetlayer didn't answer: {e}")))?;
        let status = res.status();
        if !status.is_success() {
            return Err(AppError::Unavailable(format!(
                "Fleetlayer answered {status}: {}",
                res.text().await.unwrap_or_default()
            )));
        }
        let v: serde_json::Value = res.json().await.unwrap_or_default();
        Ok(v.get("closed").and_then(serde_json::Value::as_bool).unwrap_or(false))
    }
}

/// What the Work tab shows: nothing configured, or Fleetlayer's pending requests and open windows.
pub async fn list() -> FleetApprovals {
    match APPROVER.get() {
        Some(a) => a.list().await,
        None => FleetApprovals::default(),
    }
}

fn approver() -> AppResult<&'static Approver> {
    APPROVER.get().ok_or_else(|| {
        AppError::NotFound("the deck has no Fleetlayer approve token (QUIETDECK_FLEETLAYER_APPROVE_TOKEN)".into())
    })
}

pub async fn decide(id: &str, d: &FleetDecide) -> AppResult<FleetDecided> {
    approver()?.decide(id, d).await
}

pub async fn close(role: &str) -> AppResult<bool> {
    approver()?.close(role).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_requests_and_open_windows_come_through_and_a_window_request_is_marked() {
        let raw: RawApprovals = serde_json::from_value(json!({"approvals": [
            {"id": "ap_1", "from": "ops@server-admin", "verb": "activate", "to": "window:admin", "status": "pending",
             "createdAt": 5, "expiresAt": null, "detail": "upgrade postgres", "once": false,
             "line": "ops on server-admin wants to open the admin window"},
            {"id": "ap_2", "from": "web@laptop", "verb": "start", "to": "node:server", "status": "approved",
             "createdAt": 4},
            {"id": "ap_3", "from": "web@laptop", "verb": "use_tool", "to": "tool:browser@laptop", "status": "pending",
             "createdAt": 3, "once": true},
        ]}))
        .unwrap();
        let a = approvals_of(raw);
        assert_eq!(a.iter().map(|x| (x.id.as_str(), x.window)).collect::<Vec<_>>(), [("ap_1", true), ("ap_3", false)]);
        assert!(a[1].once);
        assert_eq!(a[0].line.as_deref(), Some("ops on server-admin wants to open the admin window"));
        assert_eq!(a[1].line, None, "an older Fleetlayer sends none");
        let w: RawWindows = serde_json::from_value(json!({"roles": [], "windows": [
            {"id": "w2", "role": "admin", "requestedBy": "ops@server-admin", "openedAt": 10, "endsAt": 20,
             "closedAt": null, "closedBy": null, "unconfirmed": [], "problem": null},
            {"id": "w1", "role": "admin", "openedAt": 1, "endsAt": 2, "closedAt": 2},
        ]}))
        .unwrap();
        let open = open_windows(w);
        assert_eq!(open.len(), 1);
        assert_eq!((open[0].id.as_str(), open[0].ends_at), ("w2", Some(20)));
    }

    #[test]
    fn the_passkey_page_comes_back_to_the_deck_with_the_request() {
        let a = Approver::new(
            "https://fl.example.com/",
            "t",
            Some("https://signin.example.com/"),
            "default",
            "https://deck.example.com",
        )
        .unwrap();
        assert_eq!(
            a.stepup_url("ap_1").unwrap(),
            "https://signin.example.com/stepup?tenant=default&approval=ap_1&return=\
             https%3A%2F%2Fdeck.example.com%2F%3Ffleet_approve%3Dap_1"
        );
        let none = Approver::new("https://fl.example.com", "t", None, "default", "https://deck.example.com").unwrap();
        assert_eq!(none.stepup_url("ap_1"), None);
    }
}
