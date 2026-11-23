//! Plain-text output. `--json` skips all of this.

use quietdeck_types::api::{Caller, DeckInfo};
use quietdeck_types::card::{Card, CardAdded};
use quietdeck_types::directory::{HitSource, SessionRow, WhoHas};
use quietdeck_types::timeline::History;
use serde::Serialize;

pub fn out<T: Serialize>(json: bool, value: &T, text: impl Fn(&T) -> String) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        let t = text(value);
        if !t.is_empty() {
            println!("{t}");
        }
    }
    Ok(())
}

fn snake<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

pub fn info(i: &DeckInfo) -> String {
    let who = match &i.caller {
        Caller::Session { session } => match &session.session_id {
            Some(id) => format!("{} (session {id})", session.address()),
            None => session.address(),
        },
        Caller::Node { node } => format!("node {node}"),
        Caller::Owner { login } => format!("owner {login}"),
    };
    let accounts: Vec<&str> = i.accounts.iter().map(|a| a.id.as_str()).collect();
    format!(
        "You are {who}.\nDeck {}; accounts: {}; boards: {}.",
        i.version,
        if accounts.is_empty() { "(any)".to_string() } else { accounts.join(", ") },
        if i.boards.is_empty() { "(none)".to_string() } else { i.boards.join(", ") }
    )
}

pub fn card_line(c: &Card) -> String {
    format!(
        "c_{}  v{}  {:<17} {:<11} {:<10} {}",
        c.id,
        c.version,
        snake(&c.state),
        snake(&c.input.section),
        c.input.account,
        c.input.subject
    )
}

pub fn card_full(c: &Card) -> String {
    let mut s = card_line(c);
    s.push_str(&format!("\n{}\nReason: {}", c.input.summary, c.input.reason));
    for a in &c.action_records {
        s.push_str(&format!("\n  a_{} {} {}", a.id, snake(&a.action.action_type()), snake(&a.status)));
        if let Some(e) = &a.error {
            s.push_str(&format!(" ({e})"));
        }
    }
    if let Some(note) = &c.recheck_note {
        s.push_str(&format!("\nRecheck: {note}"));
    }
    s
}

pub fn card_added(a: &CardAdded) -> String {
    let mut s = format!(
        "{} c_{} (version {}).",
        if a.replaced { "Updated" } else { "Posted" },
        a.card.id,
        a.card.version
    );
    if let Some(m) = &a.auto {
        s.push_str(&format!(" Automatic: {} ({}).", if m.allow { "allowed" } else { "denied" }, m.reason));
    }
    s
}

pub fn session_line(r: &SessionRow) -> String {
    let status = if r.info.running { snake(&r.info.status) } else { "ended".into() };
    let offline = if r.node_online { "" } else { " [node offline]" };
    format!(
        "{:<28} {:<8} {}{}",
        r.address(),
        status,
        r.sentence.as_deref().unwrap_or("-"),
        offline
    )
}

pub fn who_has(w: &WhoHas) -> String {
    let mut lines: Vec<String> = w
        .hits
        .iter()
        .map(|h| {
            format!(
                "{}@{} ({})  {}  last {}",
                h.tab.as_deref().unwrap_or(&h.session_id),
                h.node,
                h.session_id,
                match h.source {
                    HitSource::Summary => "summary",
                    HitSource::Transcript => "transcript",
                },
                h.last_mentioned_at.map(|t| t.to_string()).unwrap_or_else(|| "?".into())
            )
        })
        .collect();
    if lines.is_empty() {
        lines.push(format!("Nobody mentions {}.", w.query));
    }
    if !w.offline_nodes.is_empty() {
        lines.push(format!("Searched by summary only (offline): {}", w.offline_nodes.join(", ")));
    }
    lines.join("\n")
}

pub fn history(h: &History) -> String {
    let mut lines = vec![format!("Since {}:", h.since)];
    for e in &h.entries {
        lines.push(format!("  {} {} {}", e.created_at, snake(&e.kind), e.text));
    }
    for c in &h.cards {
        lines.push(format!("  {}", card_line(c)));
    }
    if h.entries.is_empty() && h.cards.is_empty() {
        lines.push("  nothing".into());
    }
    lines.join("\n")
}
