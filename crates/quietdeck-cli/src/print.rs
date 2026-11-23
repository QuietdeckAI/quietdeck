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
        Caller::Owner { device } => format!("the owner, on {device}"),
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
    let mut s =
        format!("{} c_{} (version {}).", if a.replaced { "Updated" } else { "Posted" }, a.card.id, a.card.version);
    if let Some(m) = &a.auto {
        s.push_str(&format!(" Automatic: {} ({}).", if m.allow { "allowed" } else { "denied" }, m.reason));
    }
    s
}

pub fn session_line(r: &SessionRow) -> String {
    let status = if r.info.running { snake(&r.info.status) } else { "ended".into() };
    let offline = if r.node_online { "" } else { " [node offline]" };
    format!("{:<28} {:<8} {}{}", r.address(), status, r.sentence.as_deref().unwrap_or("-"), offline)
}

pub fn session_detail(d: &quietdeck_types::directory::SessionDetail) -> String {
    let r = &d.session;
    let mut out =
        vec![session_line(r), format!("session {} in {}", r.info.session_id, r.info.cwd.as_deref().unwrap_or("?"))];
    if let Some(c) = &r.info.resume_command {
        out.push(format!("resume: {c}"));
    }
    if let Some(p) = &r.page {
        out.push(String::new());
        out.push(p.clone());
    }
    if !d.timeline.is_empty() {
        out.push(String::new());
        for t in &d.timeline {
            let at = t.activity_to.to_zoned_datetime(jiff::tz::TimeZone::system());
            out.push(format!("{}  {}", at.strftime("%Y-%m-%d %H:%M"), t.sentence));
        }
    }
    out.join("\n")
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
    for p in &w.partial {
        lines.push(format!("{}: out of time, searched transcripts written since {}", p.node, p.searched_back_to));
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
    for c in &h.comments {
        lines.push(format!("  The person commented on \"{}\": {}", c.on, c.text));
    }
    if h.entries.is_empty() && h.cards.is_empty() && h.comments.is_empty() {
        lines.push("  nothing".into());
    }
    lines.join("\n")
}

pub fn rules(v: &quietdeck_types::rules::RulesView) -> String {
    use quietdeck_types::rules::{RuleMode, RuleStatus};
    let mut lines = Vec::new();
    for r in &v.rules {
        let what = match r.mode {
            RuleMode::Fuzzy => format!("fuzzy: {}", r.instruction.as_deref().unwrap_or("")),
            RuleMode::Strict => {
                let c = &r.criteria;
                let mut parts = vec![];
                parts.extend(c.account.as_ref().map(|a| format!("account {a}")));
                parts.extend(c.from_address.as_ref().map(|a| format!("from {a}")));
                parts.extend(c.from_domain.as_ref().map(|d| format!("from @{d}")));
                parts.extend(c.subject_contains.as_ref().map(|s| format!("subject contains {s:?}")));
                parts.extend(c.list_id.as_ref().map(|l| format!("List-Id {l}")));
                format!("strict: {}", parts.join(", "))
            }
        };
        let types: Vec<&str> = r.action_template.types.iter().map(|t| t.as_str()).collect();
        let target = r.action_template.target.as_ref().map(|t| format!(" to {}", t.address())).unwrap_or_default();
        let status = match r.status {
            RuleStatus::Active => String::new(),
            RuleStatus::Disabled => " [off]".into(),
            RuleStatus::Archived => format!(" [archived: {}]", r.archived_reason.as_deref().unwrap_or("")),
        };
        let domains = if r.known_sender_domains.is_empty() {
            String::new()
        } else {
            format!(" (senders: {})", r.known_sender_domains.join(", "))
        };
        lines.push(format!("g_{} {what}{domains} -> {}{target}{status}", r.id, types.join(", ")));
    }
    if lines.is_empty() {
        lines.push("No rules yet: the person grants them on the deck's Rules screen.".into());
    }
    lines.push(String::new());
    lines.push("Autonomy policies:".into());
    for p in &v.policies {
        let types: Vec<&str> = p.automatic.iter().map(|t| t.as_str()).collect();
        let scope: Vec<String> =
            p.dirs.iter().cloned().chain(p.accounts.iter().map(|a| format!("account {a}"))).collect();
        lines.push(format!(
            "  {} ({}): {}",
            p.name,
            scope.join(", "),
            if types.is_empty() { "nothing".into() } else { types.join(", ") }
        ));
    }
    lines.push(format!("Automatic resumes and new sessions: at most {} a day.", v.session_starts_per_day));
    lines.join("\n")
}

pub fn released(r: &[quietdeck_types::card::Released]) -> String {
    if r.is_empty() {
        return "No decisions are waiting.".into();
    }
    let mut s = String::new();
    for d in r {
        s.push_str(&format!("{}\nDecision: {}", card_line(&d.card), snake(&d.decision.kind)));
        if let Some(t) = &d.decision.text {
            s.push_str(&format!(" \"{t}\""));
        }
        for a in &d.actions {
            s.push_str(&format!(
                "\n  a_{} {}\n    {}",
                a.id,
                snake(&a.action.action_type()),
                serde_json::to_string(&a.action).unwrap_or_default()
            ));
        }
        s.push_str("\n\n");
    }
    s.push_str("Run `quietdeck action start <id>` before each action, then report every result.");
    s
}

pub fn receipts(r: &[quietdeck_types::decision::ReportReceipt]) -> String {
    r.iter()
        .map(|x| match &x.error {
            None => format!("a_{} recorded.", x.action_id),
            Some(e) => format!("a_{} refused: {e}", x.action_id),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn lease_line(l: &quietdeck_types::lease::Lease) -> String {
    let what = match l.params.get("query").and_then(|q| q.as_str()) {
        Some(q) => format!("{} on {}: {q}", l.kind, l.resource),
        None => format!("{} on {}", l.kind, l.resource),
    };
    let mut line =
        format!("lease {} {:<17} {what} (held by {}@{})", l.id, snake(&l.state), l.holder.tab, l.holder.node);
    if let Some(p) = l.queue_position {
        line.push_str(&format!(" [queue position {p}]"));
    }
    if let Some(c) = l.card_id
        && l.state == quietdeck_types::lease::LeaseState::Proposed
    {
        line.push_str(&format!(" [card c_{c}]"));
    }
    if let Some(r) = &l.end_reason {
        line.push_str(&format!(" [{r}]"));
    }
    line
}

pub fn leases(v: &quietdeck_types::lease::LeasesView) -> String {
    let mut out: Vec<String> = v.leases.iter().map(lease_line).collect();
    if out.is_empty() {
        out.push("No leases.".into());
    }
    let approved: Vec<String> = v
        .patterns
        .iter()
        .filter(|p| p.revoked_at.is_none())
        .map(|p| format!("  {} on {}: {}", p.kind, p.resource, p.pattern))
        .collect();
    if !approved.is_empty() {
        out.push("Approved filters (any session may re-create them):".into());
        out.extend(approved);
    }
    out.join("\n")
}
