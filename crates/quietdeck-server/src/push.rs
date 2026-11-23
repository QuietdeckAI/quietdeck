//! Web push: the devices that take pushes, and sending one push to all of them. Pushes come only from
//! `quietdeck ping`. The payload is Declarative Web Push, which Safari shows without running the service worker; the
//! service worker reads the same JSON in browsers that don't support it. Nothing is fetched when a push arrives.

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use quietdeck_types::api::{PushDevice, PushSubscribe};
use serde_json::json;
use sha2::{Digest, Sha256};
use web_push_native::jwt_simple::algorithms::{ECDSAP256PublicKeyLike, ES256KeyPair};
use web_push_native::{Auth, WebPushBuilder, p256::PublicKey};

use crate::db::{Db, now_ms, ts, ts_opt};

/// How long a push service keeps trying to deliver a push to a device that's offline.
const TTL: Duration = Duration::from_secs(12 * 3600);

pub struct Pusher {
    key: ES256KeyPair,
    public_key: String,
    contact: String,
    http: reqwest::Client,
}

impl Pusher {
    /// From the deck's VAPID keys (`quietdeck-server vapid-keys`). `subject` is the contact push services may use, a
    /// `mailto:` or `https:` URL: the deck's own URL unless set.
    pub fn new(public: &str, private: &str, subject: Option<&str>, public_url: &str) -> anyhow::Result<Pusher> {
        let bytes = B64
            .decode(private.trim())
            .map_err(|e| anyhow::anyhow!("QUIETDECK_VAPID_PRIVATE_KEY isn't base64url: {e}"))?;
        let key = ES256KeyPair::from_bytes(&bytes)
            .map_err(|e| anyhow::anyhow!("QUIETDECK_VAPID_PRIVATE_KEY isn't a P-256 private key: {e}"))?;
        let derived = B64.encode(key.public_key().public_key().to_bytes_uncompressed());
        if derived != public.trim() {
            anyhow::bail!("QUIETDECK_VAPID_PUBLIC_KEY doesn't match QUIETDECK_VAPID_PRIVATE_KEY");
        }
        let ok = |c: &str| c.starts_with("mailto:") || c.starts_with("https://");
        let contact = match subject {
            Some(s) if ok(s) => s.to_string(),
            Some(s) => anyhow::bail!("QUIETDECK_VAPID_SUBJECT must be a mailto: or https:// URL, not {s}"),
            None if ok(public_url) => public_url.to_string(),
            None => anyhow::bail!(
                "QUIETDECK_PUBLIC_URL isn't https, so push needs a contact: set QUIETDECK_VAPID_SUBJECT to \
                 mailto:<your address>"
            ),
        };
        let http = reqwest::Client::builder().timeout(Duration::from_secs(10)).build()?;
        Ok(Pusher { key, public_key: derived, contact, http })
    }

    pub fn public_key(&self) -> &str {
        &self.public_key
    }

    /// Send one push to every device. Devices their push service says are gone are removed.
    pub async fn send_all(&self, db: &Db, m: &Message) -> anyhow::Result<Sent> {
        let rows: Vec<(String, String, String)> =
            sqlx::query_as("SELECT endpoint, p256dh, auth FROM push_subscriptions ORDER BY created_at")
                .fetch_all(db)
                .await?;
        let body = m.payload();
        let topic = m.topic.as_deref().map(topic);
        let mut tasks = tokio::task::JoinSet::new();
        for (endpoint, p256dh, auth) in rows.iter().cloned() {
            let request = self.request(&endpoint, &p256dh, &auth, &body, topic.as_deref());
            let http = self.http.clone();
            tasks.spawn(async move {
                let outcome = match request {
                    Ok(r) => match http.execute(r).await {
                        Ok(resp) => Outcome::Status(resp.status().as_u16()),
                        Err(e) => Outcome::Failed(e.to_string()),
                    },
                    Err(e) => Outcome::Failed(e),
                };
                (endpoint, outcome)
            });
        }
        let mut delivered = 0;
        while let Some(done) = tasks.join_next().await {
            let (endpoint, outcome) = done?;
            match outcome {
                Outcome::Status(s) if (200..300).contains(&s) => {
                    delivered += 1;
                    sqlx::query("UPDATE push_subscriptions SET last_ok_at = ? WHERE endpoint = ?")
                        .bind(now_ms())
                        .bind(&endpoint)
                        .execute(db)
                        .await?;
                }
                Outcome::Status(404 | 410) => {
                    tracing::info!(service = %host(&endpoint), "a device's push subscription has ended: removed");
                    remove(db, &endpoint).await?;
                }
                Outcome::Status(s) => tracing::warn!(service = %host(&endpoint), status = s, "a push wasn't taken"),
                Outcome::Failed(e) => tracing::warn!(service = %host(&endpoint), error = %e, "a push failed"),
            }
        }
        Ok(Sent { delivered, devices: rows.len() })
    }

    fn request(
        &self,
        endpoint: &str,
        p256dh: &str,
        auth: &str,
        body: &[u8],
        topic: Option<&str>,
    ) -> Result<reqwest::Request, String> {
        let public = B64.decode(p256dh).map_err(|e| e.to_string())?;
        let auth = B64.decode(auth).map_err(|e| e.to_string())?;
        if auth.len() != 16 {
            return Err("the subscription's auth secret isn't 16 bytes".into());
        }
        let builder = WebPushBuilder::new(
            endpoint.parse().map_err(|e: http::uri::InvalidUri| e.to_string())?,
            PublicKey::from_sec1_bytes(&public).map_err(|e| e.to_string())?,
            Auth::clone_from_slice(&auth),
        )
        .with_valid_duration(TTL)
        .with_vapid(&self.key, &self.contact);
        let mut request = builder.build(body.to_vec()).map_err(|e| e.to_string())?;
        let headers = request.headers_mut();
        headers.insert("Urgency", http::HeaderValue::from_static("high"));
        if let Some(t) = topic {
            headers.insert("Topic", http::HeaderValue::from_str(t).map_err(|e| e.to_string())?);
        }
        reqwest::Request::try_from(request).map_err(|e| e.to_string())
    }
}

enum Outcome {
    Status(u16),
    Failed(String),
}

/// Only the push service's host goes in the log: an endpoint URL is a credential for that device.
fn host(endpoint: &str) -> String {
    endpoint.parse::<http::Uri>().ok().and_then(|u| u.host().map(str::to_string)).unwrap_or_default()
}

/// A push's text, where tapping it goes, and the thread it's about (repeats for one thread replace each other).
pub struct Message {
    pub title: String,
    pub body: String,
    pub navigate: String,
    pub topic: Option<String>,
}

impl Message {
    fn payload(&self) -> Vec<u8> {
        let mut n = json!({ "title": self.title, "body": self.body, "navigate": self.navigate });
        if let Some(t) = &self.topic {
            n["tag"] = json!(topic(t));
        }
        serde_json::to_vec(&json!({ "web_push": 8030, "notification": n })).expect("JSON serialises")
    }
}

/// RFC 8030's Topic header: at most 32 characters of base64url. A hash of the thread fits any thread key.
fn topic(key: &str) -> String {
    B64.encode(&Sha256::digest(key.as_bytes())[..24])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sent {
    pub delivered: usize,
    pub devices: usize,
}

pub async fn subscribe(db: &Db, s: &PushSubscribe) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO push_subscriptions (endpoint, p256dh, auth, device, created_at) VALUES (?, ?, ?, ?, ?) \
         ON CONFLICT (endpoint) DO UPDATE SET p256dh = excluded.p256dh, auth = excluded.auth, \
         device = COALESCE(excluded.device, device)",
    )
    .bind(&s.endpoint)
    .bind(&s.p256dh)
    .bind(&s.auth)
    .bind(&s.device)
    .bind(now_ms())
    .execute(db)
    .await?;
    Ok(())
}

/// Whether a subscription's keys are ones a push can be encrypted to.
pub fn check_keys(s: &PushSubscribe) -> Result<(), String> {
    let public = B64.decode(&s.p256dh).map_err(|_| "p256dh isn't base64url".to_string())?;
    PublicKey::from_sec1_bytes(&public).map_err(|_| "p256dh isn't a P-256 public key".to_string())?;
    match B64.decode(&s.auth) {
        Ok(a) if a.len() == 16 => Ok(()),
        _ => Err("auth isn't a 16-byte base64url secret".into()),
    }
}

pub async fn devices(db: &Db) -> anyhow::Result<Vec<PushDevice>> {
    let rows: Vec<(String, Option<String>, i64, Option<i64>)> =
        sqlx::query_as("SELECT endpoint, device, created_at, last_ok_at FROM push_subscriptions ORDER BY created_at")
            .fetch_all(db)
            .await?;
    Ok(rows
        .into_iter()
        .map(|(endpoint, device, created_at, last_ok_at)| PushDevice {
            endpoint,
            device,
            created_at: ts(created_at),
            last_ok_at: ts_opt(last_ok_at),
        })
        .collect())
}

pub async fn remove(db: &Db, endpoint: &str) -> anyhow::Result<bool> {
    let r = sqlx::query("DELETE FROM push_subscriptions WHERE endpoint = ?").bind(endpoint).execute(db).await?;
    Ok(r.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Bytes;
    use axum::http::HeaderMap;
    use std::sync::{Arc, Mutex};

    fn keys() -> (String, String) {
        let kp = ES256KeyPair::generate();
        (B64.encode(kp.public_key().public_key().to_bytes_uncompressed()), B64.encode(kp.to_bytes()))
    }

    #[test]
    fn the_keys_must_match_and_the_subject_must_be_a_url() {
        let (public, private) = keys();
        let p = Pusher::new(&public, &private, None, "https://deck.example.com").unwrap();
        assert_eq!(p.public_key(), public);
        let (other, _) = keys();
        assert!(Pusher::new(&other, &private, None, "https://deck.example.com").is_err());
        assert!(Pusher::new(&public, &private, Some("someone"), "https://deck.example.com").is_err());
        assert!(Pusher::new(&public, &private, None, "http://localhost:8080").is_err());
        assert!(Pusher::new(&public, &private, Some("mailto:me@example.com"), "http://localhost:8080").is_ok());
        assert!(Pusher::new(&public, "not base64!", None, "https://deck.example.com").is_err());
    }

    #[test]
    fn a_topic_is_32_base64url_characters_for_any_thread() {
        let t = topic(&"x".repeat(1000));
        assert_eq!(t.len(), 32);
        assert!(t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        assert_eq!(topic("personal:abc"), topic("personal:abc"));
        assert_ne!(topic("personal:abc"), topic("personal:abd"));
    }

    /// A push service on localhost: it records each push and answers with the status the test sets per path.
    async fn push_service() -> (String, Arc<Mutex<Vec<(String, HeaderMap, Bytes)>>>) {
        let got: Arc<Mutex<Vec<(String, HeaderMap, Bytes)>>> = Arc::default();
        let g = got.clone();
        let app = axum::Router::new().route(
            "/{device}",
            axum::routing::post(
                move |axum::extract::Path(device): axum::extract::Path<String>, h: HeaderMap, b: Bytes| {
                    let g = g.clone();
                    async move {
                        g.lock().unwrap().push((device.clone(), h, b));
                        if device == "gone" { axum::http::StatusCode::GONE } else { axum::http::StatusCode::CREATED }
                    }
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (url, got)
    }

    #[tokio::test]
    async fn a_push_reaches_every_device_encrypted_and_ended_subscriptions_go() {
        let db = crate::db::memory().await;
        let (url, got) = push_service().await;
        let (public, private) = keys();
        let pusher = Pusher::new(&public, &private, Some("mailto:me@example.com"), "https://deck.example.com").unwrap();
        // The browser's side: its key pair and auth secret.
        let ua = p256::SecretKey::from_slice(&crate::auth::random_bytes::<32>()).unwrap();
        let ua_public = B64
            .encode(p256::elliptic_curve::sec1::ToEncodedPoint::to_encoded_point(&ua.public_key(), false).as_bytes());
        let auth = [7u8; 16];
        for device in ["phone", "gone"] {
            // Subscriptions are https in the API; the store doesn't care, which lets the test use plain HTTP.
            let s = PushSubscribe {
                endpoint: format!("{url}/{device}"),
                p256dh: ua_public.clone(),
                auth: B64.encode(auth),
                device: Some(device.into()),
            };
            check_keys(&s).unwrap();
            subscribe(&db, &s).await.unwrap();
        }
        let m = Message {
            title: "Reply from the bank".into(),
            body: "They need the form today".into(),
            navigate: "https://deck.example.com/rules?ping=1".into(),
            topic: Some("personal:t1".into()),
        };
        let sent = pusher.send_all(&db, &m).await.unwrap();
        assert_eq!(sent, Sent { delivered: 1, devices: 2 });
        let left = devices(&db).await.unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].device.as_deref(), Some("phone"));
        assert!(left[0].last_ok_at.is_some());

        let got = got.lock().unwrap();
        let (_, headers, body) = got.iter().find(|(d, ..)| d == "phone").unwrap();
        assert_eq!(headers["urgency"], "high");
        assert_eq!(headers["topic"], topic("personal:t1").as_str());
        assert!(headers["authorization"].to_str().unwrap().starts_with("vapid t="));
        let plain = web_push_native::decrypt(body.to_vec(), &ua, &Auth::clone_from_slice(&auth)).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&plain).unwrap();
        assert_eq!(v["web_push"], 8030);
        assert_eq!(v["notification"]["title"], "Reply from the bank");
        assert_eq!(v["notification"]["navigate"], "https://deck.example.com/rules?ping=1");
        assert_eq!(v["notification"]["tag"], topic("personal:t1"));
    }
}
