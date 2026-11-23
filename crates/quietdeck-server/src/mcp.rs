//! The MCP endpoint at `/mcp` (rmcp's streamable HTTP server). Every tool takes the same types as the REST API and
//! calls the same functions. Callers authenticate the same way: Claude Code's `headersHelper` runs
//! `quietdeck auth headers`, which adds the session token from the local node.

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ServerCapabilities, ServerConfig};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData, RoleServer, ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use quietdeck_types::SessionRef;
use quietdeck_types::api::Caller;
use quietdeck_types::card::{ActionReport, CardInput, CardResolution};
use quietdeck_types::directory::SessionTarget;
use quietdeck_types::relay::MsgSend;
use quietdeck_types::timeline::TimelineAdd;

use crate::error::AppError;
use crate::ops;
use crate::state::AppState;
use crate::store::{cards, sessions};

#[derive(Clone)]
pub struct Mcp {
    state: AppState,
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CardResolveArgs {
    /// The card's ID.
    pub id: i64,
    pub resolution: CardResolution,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SessionsArgs {
    /// Only this node's sessions.
    #[serde(default)]
    pub node: Option<String>,
    /// Include sessions that aren't running.
    #[serde(default)]
    pub all: bool,
    /// Only sessions active since a date (`2026-05-23`) or a number of days back (`180d`), running or not.
    #[serde(default)]
    pub since: Option<String>,
    /// At most this many (default 200, or 5,000 with `since`).
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SessionShowArgs {
    /// A session ID, `tab@machine`, or a tab name.
    pub session: String,
    /// Include every summary point the session has, oldest first.
    #[serde(default)]
    pub timeline: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchArgs {
    /// Words to find (all of them), or an ID such as a story or invoice number.
    pub q: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ActionStartArgs {
    pub action_id: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ActionsReportArgs {
    /// One entry per action: `{"action_id": 1, "result": "done", "outcome": {...}}` or
    /// `{"action_id": 2, "result": "failed", "error": "..."}`.
    pub reports: Vec<ActionReport>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HistoryArgs {
    /// `last` (since this session last asked) or an RFC 3339 time. Default `last`.
    #[serde(default)]
    pub since: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CardsArgs {
    /// Only cards still on the Inbox.
    #[serde(default)]
    pub open: bool,
    #[serde(default)]
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NoArgs {}

fn ok<T: Serialize>(value: T) -> Result<CallToolResult, ErrorData> {
    let v = serde_json::to_value(value).map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
    // Structured content must be an object; wrap lists.
    let v = if v.is_object() { v } else { serde_json::json!({ "items": v }) };
    Ok(CallToolResult::structured(v))
}

fn fail(e: AppError) -> Result<CallToolResult, ErrorData> {
    if let AppError::Internal(err) = &e {
        tracing::error!(error = %err, "internal error in an MCP tool");
    }
    let message = match &e {
        AppError::Internal(_) => "internal error; the deck's log has the details".to_string(),
        other => other.to_string(),
    };
    Ok(CallToolResult::structured_error(serde_json::json!({ "error": message })))
}

macro_rules! tool_try {
    ($e:expr) => {
        match $e {
            Ok(v) => ok(v),
            Err(e) => fail(e.into()),
        }
    };
}

impl Mcp {
    pub fn new(state: AppState) -> Self {
        Mcp { state, tool_router: Self::tool_router() }
    }

    async fn caller(&self, ctx: &RequestContext<RoleServer>) -> Result<Caller, AppError> {
        let parts = ctx
            .extensions
            .get::<axum::http::request::Parts>()
            .ok_or_else(|| AppError::Unauthorized("no HTTP request on this MCP call".into()))?;
        crate::auth::resolve(&self.state, parts).await
    }

    async fn agent(&self, ctx: &RequestContext<RoleServer>) -> Result<SessionRef, AppError> {
        match self.caller(ctx).await? {
            Caller::Session { session } => Ok(session),
            _ => Err(AppError::Forbidden(
                "this tool needs a session token: set headersHelper to `quietdeck auth headers`".into(),
            )),
        }
    }
}

#[tool_router]
impl Mcp {
    #[tool(description = "Who you are to the deck, its accounts, boards and settings.")]
    async fn info(&self, _: Parameters<NoArgs>, ctx: RequestContext<RoleServer>) -> Result<CallToolResult, ErrorData> {
        let r = async { ops::info(&self.state, self.caller(&ctx).await?).await }.await;
        tool_try!(r)
    }

    #[tool(description = "Post a card for one email thread: summary, facts, section, reason and typed actions. \
        A thread's open card gets a new version. Never put email bodies on a card.")]
    async fn card_add(
        &self,
        Parameters(input): Parameters<CardInput>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let r = async { ops::add_card(&self.state, &self.agent(&ctx).await?, input).await }.await;
        tool_try!(r)
    }

    #[tool(description = "Report your recheck of a card: archived in Gmail, handled elsewhere (with a note the \
        person reads), or rechecked and still standing.")]
    async fn card_resolve(
        &self,
        Parameters(args): Parameters<CardResolveArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let r = async {
            self.agent(&ctx).await?;
            garde::Validate::validate(&args.resolution).map_err(crate::error::bad)?;
            cards::resolve(&self.state.db, args.id, &args.resolution).await
        }
        .await;
        tool_try!(r)
    }

    #[tool(description = "List cards, newest first.")]
    async fn cards(
        &self,
        Parameters(args): Parameters<CardsArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let r = async {
            self.caller(&ctx).await?;
            let f = cards::ListFilter { open: args.open, account: args.account, ..Default::default() };
            Ok::<_, AppError>(cards::list(&self.state.db, &f).await?)
        }
        .await;
        tool_try!(r)
    }

    #[tool(description = "Every session on every machine, with its tab, status, summary and links.")]
    async fn sessions(
        &self,
        Parameters(args): Parameters<SessionsArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let r = async {
            self.caller(&ctx).await?;
            let f = sessions::ListFilter { node: args.node, all: args.all, since: args.since, limit: args.limit };
            Ok::<_, AppError>(sessions::list(&self.state.db, &self.state.hub, &f).await?)
        }
        .await;
        tool_try!(r)
    }

    #[tool(
        description = "One session, by ID, tab@machine or tab, with every earlier one-line update when timeline is set."
    )]
    async fn session_show(
        &self,
        Parameters(args): Parameters<SessionShowArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let r = async {
            self.caller(&ctx).await?;
            crate::ops::show_session(&self.state, &args.session, args.timeline).await
        }
        .await;
        tool_try!(r)
    }

    #[tool(description = "Search every session's summaries.")]
    async fn sessions_search(
        &self,
        Parameters(args): Parameters<SearchArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let r = async {
            self.caller(&ctx).await?;
            Ok::<_, AppError>(sessions::search(&self.state.db, &self.state.hub, &args.q, 20).await?)
        }
        .await;
        tool_try!(r)
    }

    #[tool(description = "Resume a session that isn't running, in a new tab on its machine, in its own working \
        directory. `session` is tab@machine, a tab name, or a session ID; with a tab name, the one that ended last.")]
    async fn session_resume(
        &self,
        Parameters(target): Parameters<SessionTarget>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let r = async {
            let caller = self.caller(&ctx).await?;
            if matches!(caller, Caller::Node { .. }) {
                return Err(AppError::Forbidden("a node can't start sessions".into()));
            }
            crate::control::resume(&self.state, &caller, target).await
        }
        .await;
        tool_try!(r)
    }

    #[tool(description = "Who has this ID? Searches every session's summaries, and the raw transcripts on every \
        connected machine. Says which machines were offline (searched by summary only).")]
    async fn who_has(
        &self,
        Parameters(args): Parameters<SearchArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let r = async {
            self.caller(&ctx).await?;
            ops::who_has(&self.state, &args.q).await
        }
        .await;
        tool_try!(r)
    }

    #[tool(description = "Send a message to a session on another machine (`tab` or `tab@node`). Fails at once when \
        that session is offline: then find another way or try again later. Use SendMessage on the same machine.")]
    async fn msg_send(
        &self,
        Parameters(msg): Parameters<MsgSend>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let r = async { ops::msg_send(&self.state, &self.agent(&ctx).await?, msg).await }.await;
        tool_try!(r)
    }

    #[tool(description = "Post a timeline entry: an automatic action you carried out, or mail a lease absorbed.")]
    async fn timeline_add(
        &self,
        Parameters(e): Parameters<TimelineAdd>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let r = async { ops::timeline_add(&self.state, &self.caller(&ctx).await?, e).await }.await;
        tool_try!(r)
    }

    #[tool(description = "Claim the person's released decisions on your cards, with each card's actions exactly as \
        approved (their edits applied). Carry them out, calling action_start before each one, then actions_report.")]
    async fn decisions_take(
        &self,
        _: Parameters<NoArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let r = async { ops::take(&self.state, &self.agent(&ctx).await?).await }.await;
        tool_try!(r)
    }

    #[tool(description = "Say your triage round is finished, so the runner may wake you again when new mail or \
        events arrive. Call it at the end of every round.")]
    async fn round_done(
        &self,
        _: Parameters<NoArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let r = async {
            let me = self.agent(&ctx).await?;
            Ok::<_, crate::error::AppError>(quietdeck_types::api::RoundDone {
                runner_told: crate::runner::round_done(&self.state, &me).await?,
            })
        }
        .await;
        tool_try!(r)
    }

    #[tool(description = "Start one approved action before you run it. Refused once the person has pressed Stop: \
        then don't run it.")]
    async fn action_start(
        &self,
        Parameters(args): Parameters<ActionStartArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let r = async { ops::action_start(&self.state, &self.agent(&ctx).await?, args.action_id).await }.await;
        tool_try!(r)
    }

    #[tool(description = "Report the results of started actions: done (with what was sent and came back) or failed \
        (with an error the person can act on).")]
    async fn actions_report(
        &self,
        Parameters(args): Parameters<ActionsReportArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let r = async { ops::actions_report(&self.state, &self.agent(&ctx).await?, args.reports).await }.await;
        tool_try!(r)
    }

    #[tool(description = "The auto-approve rules the person granted (active, archived and off), and each project's \
        autonomy policy. Ask for an automatic action with card_add's auto_grant set to an active rule's id.")]
    async fn rules_list(
        &self,
        _: Parameters<NoArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let r = async {
            self.agent(&ctx).await?;
            ops::rules_view(&self.state).await
        }
        .await;
        tool_try!(r)
    }

    #[tool(description = "Check whether a card may be carried out without the person under a rule, without posting \
        it: allow or deny, with the reason. card_add with auto_grant runs the same check for real.")]
    async fn rules_match(
        &self,
        Parameters(req): Parameters<quietdeck_types::rules::MatchRequest>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let r = async {
            self.agent(&ctx).await?;
            ops::rules_match(&self.state, req).await
        }
        .await;
        tool_try!(r)
    }

    #[tool(description = "What happened since you last asked: timeline entries, cards that changed, and the \
        person's comments for you.")]
    async fn history(
        &self,
        Parameters(args): Parameters<HistoryArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let r = async {
            let me = self.agent(&ctx).await?;
            ops::history(&self.state, &me, args.since.as_deref().unwrap_or("last")).await
        }
        .await;
        tool_try!(r)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Mcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(rmcp::model::Implementation::new("quietdeck", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Quietdeck: cards for the person to approve, the sessions directory, the cross-machine message relay \
             and the timeline. The same commands as the quietdeck CLI.",
            )
    }
}

/// The `/mcp` service. The Host check accepts the deck's public host only.
pub fn service(state: AppState) -> StreamableHttpService<Mcp, LocalSessionManager> {
    let host = url_host(&state.public_url);
    let mut config = StreamableHttpServerConfig::default();
    config = match host {
        Some(h) => config.with_allowed_hosts([h.clone(), format!("{h}:443"), "localhost".into(), "127.0.0.1".into()]),
        None => config.disable_allowed_hosts(),
    };
    StreamableHttpService::new(move || Ok(Mcp::new(state.clone())), Default::default(), config)
}

fn url_host(url: &str) -> Option<String> {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let host = rest.split(['/', ':']).next()?;
    (!host.is_empty()).then(|| host.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn hosts_come_from_the_public_url() {
        assert_eq!(super::url_host("https://deck.example.ts.net").as_deref(), Some("deck.example.ts.net"));
        assert_eq!(super::url_host("http://localhost:8080/x").as_deref(), Some("localhost"));
    }
}
