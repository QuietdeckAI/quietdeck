//! quietdeck-server: the deck. `serve` runs it; the other subcommands are for setting it up.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use clap::{Parser, Subcommand};
use quietdeck_server::config::{DataEnv, Env};
use quietdeck_server::{auth, db, passkeys, serve, store};

#[derive(Parser)]
#[command(name = "quietdeck-server", version, about = "Quietdeck's server: the deck, its web app and API")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the deck: the app on QUIETDECK_LISTEN, the webhook receiver on QUIETDECK_HOOKS_LISTEN.
    Serve(Box<Env>),
    /// Print a new Web Push (VAPID) key pair for QUIETDECK_VAPID_PUBLIC_KEY and QUIETDECK_VAPID_PRIVATE_KEY.
    VapidKeys,
    /// Print a one-time code for `quietdeck node enroll`, without the web app (run it in the deck's container).
    EnrolmentCode(DataEnv),
    /// Print a one-time code for registering a passkey without being signed in: the first one, or a new one when
    /// every passkey is lost (run it in the deck's container).
    SetupCode(DataEnv),
    /// Print a session token for a made-up tab, for filling a development deck with test cards and reports through
    /// the API (curl -H "Authorization: Bearer ..."). Only with QUIETDECK_DEV_TRUST_LOCAL set, as a dev deck runs.
    DevToken {
        #[command(flatten)]
        data: DataEnv,
        #[arg(long, default_value = "dev")]
        node: String,
        #[arg(long)]
        tab: String,
        #[arg(long, default_value_t = 24)]
        hours: i64,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,sqlx=warn".into()),
        )
        .init();
    match Cli::parse().command {
        Command::Serve(env) => serve(*env).await,
        Command::VapidKeys => {
            let (public, private) = vapid_keys();
            println!("QUIETDECK_VAPID_PUBLIC_KEY={public}");
            println!("QUIETDECK_VAPID_PRIVATE_KEY={private}");
            Ok(())
        }
        Command::SetupCode(data) => {
            let db = db::open(&data.db_path()).await?;
            let code = passkeys::create_setup_code(&db).await?;
            println!("{code}");
            eprintln!("Valid for 15 minutes, once: enter it on the deck's sign-in page to register a passkey.");
            Ok(())
        }
        Command::DevToken { data, node, tab, hours } => {
            let dev = std::env::var("QUIETDECK_DEV_TRUST_LOCAL").is_ok_and(|v| matches!(v.as_str(), "1" | "true"));
            anyhow::ensure!(dev, "dev-token is for development decks: set QUIETDECK_DEV_TRUST_LOCAL=1");
            let db = db::open(&data.db_path()).await?;
            let key = auth::signing_key(&db).await?;
            let claims = quietdeck_types::node::TokenClaims {
                node,
                session_id: Some(format!("dev-{tab}")),
                tab: Some(tab),
                address: None,
                exp: jiff::Timestamp::now().as_second() + hours * 3600,
            };
            println!("{}", auth::mint_session_token(&key, &claims));
            Ok(())
        }
        Command::EnrolmentCode(data) => {
            let db = db::open(&data.db_path()).await?;
            let code = store::nodes::create_enrolment_code(&db).await?;
            println!("{}", code.code);
            eprintln!(
                "Valid for 15 minutes, once: quietdeck node enroll {} --deck <deck URL> --name <machine>",
                code.code
            );
            Ok(())
        }
    }
}

fn vapid_keys() -> (String, String) {
    loop {
        // A random 32-byte scalar is a valid P-256 key unless it's zero or past the group order (vanishingly rare).
        if let Ok(secret) = p256::SecretKey::from_slice(&auth::random_bytes::<32>()) {
            let public = p256::elliptic_curve::sec1::ToEncodedPoint::to_encoded_point(&secret.public_key(), false);
            return (B64.encode(public.as_bytes()), B64.encode(secret.to_bytes()));
        }
    }
}
