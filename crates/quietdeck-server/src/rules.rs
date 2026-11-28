//! `rules match`: may a card be carried out without the person? The deck checks strict criteria itself; for fuzzy
//! rules the agent judged the match, and the deck checks the grant, the sender's domain, the template and the
//! projects' policies. Automatic resumes and new sessions are capped at a few a day.

use quietdeck_types::action::{Action, ActionType};
use quietdeck_types::card::CardInput;
use quietdeck_types::rules::{MatchResult, Rule, RuleMode, RuleStatus};

use crate::config::DeckConfig;
use crate::db::Db;
use crate::error::AppResult;
use crate::store::{rules, sessions};

/// Automatic resumes and `new_session` actions allowed in any 24 hours.
pub const SESSION_STARTS_PER_DAY: u32 = 3;

/// A domain, or a subdomain of it.
fn domain_matches(domain: &str, allowed: &str) -> bool {
    let (domain, allowed) = (domain.to_lowercase(), allowed.to_lowercase());
    domain == allowed || domain.ends_with(&format!(".{allowed}"))
}

/// The session an action forwards to, resumes or starts, with any directory the action names.
fn target_of(a: &Action) -> Option<(&str, &str, Option<&str>)> {
    match a {
        Action::Forward { to, .. } => Some((to.node.as_str(), to.tab.as_str(), None)),
        Action::Resume { node, tab, cwd, .. } | Action::NewSession { node, tab, cwd, .. } => {
            Some((node.as_str(), tab.as_str(), Some(cwd.as_str())))
        }
        _ => None,
    }
}

/// The checks that need only the rule and the card.
pub fn judge(rule: &Rule, card: &CardInput) -> Result<(), String> {
    match rule.status {
        RuleStatus::Active => {}
        RuleStatus::Archived => {
            return Err(format!(
                "rule {} is archived ({})",
                rule.id,
                rule.archived_reason.as_deref().unwrap_or("by the person")
            ));
        }
        RuleStatus::Disabled => return Err(format!("rule {} is turned off", rule.id)),
    }
    if card.actions.is_empty() {
        return Err("the card proposes no actions".into());
    }
    let domain = card.sender.as_ref().and_then(|s| s.domain()).map(str::to_lowercase);
    match rule.mode {
        RuleMode::Strict => {
            let c = &rule.criteria;
            if c.account.as_deref().is_some_and(|a| a != card.account) {
                return Err(format!(
                    "the rule is for account {}, not {}",
                    c.account.as_deref().unwrap_or(""),
                    card.account
                ));
            }
            if c.from_address.is_some() || c.from_domain.is_some() {
                let Some(sender) = &card.sender else { return Err("the card names no sender".into()) };
                if let Some(want) = &c.from_address
                    && !sender.address.eq_ignore_ascii_case(want)
                {
                    return Err(format!("the sender {} isn't {want}", sender.address));
                }
                if let Some(want) = &c.from_domain
                    && !domain.as_deref().is_some_and(|d| domain_matches(d, want))
                {
                    return Err(format!("the sender {} isn't from {want}", sender.address));
                }
                if !card.sender_auth.dmarc_pass && !card.sender_auth.dkim_pass {
                    return Err("neither DMARC nor DKIM passed for this message, so its sender can't be trusted".into());
                }
            }
            if let Some(want) = &c.subject_contains
                && !card.subject.to_lowercase().contains(&want.to_lowercase())
            {
                return Err(format!("the subject doesn't contain {want:?}"));
            }
            if let Some(want) = &c.list_id {
                let trim = |s: &str| s.trim().trim_start_matches('<').trim_end_matches('>').to_lowercase();
                if card.list_id.as_deref().map(trim) != Some(trim(want)) {
                    return Err(format!("the List-Id isn't {want}"));
                }
            }
        }
        RuleMode::Fuzzy => {
            let Some(d) = &domain else { return Err("the card names no sender".into()) };
            if !rule.known_sender_domains.iter().any(|k| domain_matches(d, k)) {
                return Err(format!("{d} isn't one of the rule's known sender domains"));
            }
        }
    }
    for a in &card.actions {
        let t = a.action_type();
        if !rule.action_template.types.contains(&t) {
            return Err(format!("the rule doesn't allow {t}"));
        }
        if let Some(acct) = a.account()
            && acct != card.account
        {
            return Err(format!("an action is on account {acct}, not the card's {}", card.account));
        }
        if let Some((node, tab, _)) = target_of(a) {
            let Some(want) = &rule.action_template.target else {
                return Err(format!("the rule names no session for {t}"));
            };
            if want.node != node || want.tab != tab {
                return Err(format!("the rule allows {t} only to {}, not {tab}@{node}", want.address()));
            }
        }
    }
    Ok(())
}

/// The project whose policy governs an action: the rule's own, else the target session's directory, else the
/// project that lists the action's (or the card's) account.
async fn project_for(
    db: &Db,
    config: &DeckConfig,
    rule: &Rule,
    a: &Action,
    card: &CardInput,
) -> AppResult<Option<String>> {
    if let Some(p) = &rule.project {
        return Ok(Some(p.clone()));
    }
    if let Some((node, tab, cwd)) = target_of(a) {
        let found = match (a, cwd) {
            (Action::Resume { session_id, .. }, _) => sessions::project_of_session(db, node, session_id).await?,
            _ => None,
        };
        let found = match (found, cwd) {
            (Some(p), _) => Some(p),
            (None, Some(cwd)) => sessions::project_for_cwd(config, cwd),
            (None, None) => sessions::project_of_tab(db, node, tab).await?,
        };
        return Ok(found);
    }
    let account = a.account().unwrap_or(&card.account);
    Ok(config.projects.iter().find(|(_, p)| p.accounts.iter().any(|x| x == account)).map(|(n, _)| n.clone()))
}

/// Everything `rules match` checks. The answer, and how many automatic session starts the card would use.
pub async fn check(db: &Db, config: &DeckConfig, card: &CardInput, grant_id: i64) -> AppResult<(MatchResult, usize)> {
    let deny = |reason: String| Ok((MatchResult { allow: false, grant_id, reason }, 0));
    let Some(rule) = rules::get(db, grant_id).await? else { return deny(format!("there's no rule {grant_id}")) };
    if let Err(reason) = judge(&rule, card) {
        return deny(reason);
    }
    for a in &card.actions {
        let t = a.action_type();
        let Some(project) = project_for(db, config, &rule, a, card).await? else {
            let place = match target_of(a) {
                Some((node, tab, _)) => format!("{tab}@{node}'s directory"),
                None => format!("account {}", a.account().unwrap_or(&card.account)),
            };
            return deny(format!("no project in deck.yaml covers {place}, so no policy allows {t} automatically"));
        };
        let Some(policy) = config.projects.get(&project) else {
            return deny(format!("project {project} isn't in deck.yaml"));
        };
        if !policy.automatic.contains(&t) {
            return deny(format!("project {project}'s policy doesn't allow {t} automatically"));
        }
    }
    let starts =
        card.actions.iter().filter(|a| matches!(a.action_type(), ActionType::Resume | ActionType::NewSession)).count();
    if starts > 0 {
        let used = rules::session_starts_in_last_day(db).await?;
        if used as usize + starts > SESSION_STARTS_PER_DAY as usize {
            return deny(format!(
                "at most {SESSION_STARTS_PER_DAY} automatic resumes and new sessions a day, and that's used up"
            ));
        }
    }
    let reason = match rule.mode {
        RuleMode::Strict => format!("rule {grant_id}'s criteria match"),
        RuleMode::Fuzzy => format!("rule {grant_id} allows it"),
    };
    Ok((MatchResult { allow: true, grant_id, reason }, starts))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use quietdeck_types::SessionRef;
    use quietdeck_types::card::{CardState, Released};
    use quietdeck_types::directory::{SessionEnd, SessionInfo};
    use quietdeck_types::rules::{RuleInput, RuleStatus, RuleStatusChange};
    use quietdeck_types::timeline::{CommentAdd, CommentTarget, TimelineKind};
    use quietdeck_types::{AgentKind, AgentStatus};
    use serde_json::json;

    use crate::ops;
    use crate::state::AppState;
    use crate::store::{decisions, sessions, timeline};

    const CONFIG: &str = "accounts:\n  - { id: personal, label: Personal }\n  - { id: work, label: Work }\n\
        projects:\n  app: { dirs: [~/code/app], automatic: [forward, archive] }\n\
        \x20 personal: { accounts: [personal], automatic: [archive] }\n\
        \x20 ops: { dirs: [~/code/ops], automatic: [forward, resume, new_session] }\n\
        triage: { node: server, tab: email triage }\n";

    async fn deck() -> AppState {
        let mut state = AppState::for_tests().await;
        state.config = Arc::new(yaml_serde::from_str(CONFIG).unwrap());
        for (id, tab, cwd) in [("s-esign", "esign cert", "/home/sam/code/app"), ("s-ops", "ops", "/home/sam/code/ops")]
        {
            let info = SessionInfo {
                session_id: id.into(),
                agent: AgentKind::Claude,
                workspace: None,
                tab: Some(tab.into()),
                address: Some("p1".into()),
                cwd: Some(cwd.into()),
                status: AgentStatus::Idle,
                status_changed_at: jiff::Timestamp::now(),
                resume_command: None,
                running: true,
                open_url: None,
                opens_in: None,
            };
            sessions::upsert(&state.db, &state.config, "server", &info).await.unwrap();
        }
        state
    }

    fn triage() -> SessionRef {
        SessionRef { node: "server".into(), tab: "email triage".into(), session_id: None }
    }

    fn esign() -> serde_json::Value {
        json!({"node": "server", "tab": "esign cert"})
    }

    async fn sectigo_rule(state: &AppState) -> i64 {
        let input: RuleInput = serde_json::from_value(json!({"mode": "strict",
            "criteria": {"account": "work", "from_domain": "Sectigo.com"},
            "action_template": {"types": ["forward"], "target": esign()}}))
        .unwrap();
        ops::rule_create(state, input).await.unwrap().id
    }

    fn card(account: &str, from: &str, dmarc: bool, actions: serde_json::Value) -> serde_json::Value {
        json!({"account": account, "thread_key": format!("t-{from}"), "subject": "Your certificate is issued",
            "summary": "s", "section": "actions", "reason": "r",
            "sender": {"address": from}, "sender_auth": {"dmarc_pass": dmarc},
            "actions": actions})
    }

    fn forward_to_esign() -> serde_json::Value {
        json!([{"type": "forward", "to": esign(), "text": "The certificate is issued."}])
    }

    async fn post(state: &AppState, mut c: serde_json::Value, grant: i64) -> quietdeck_types::card::CardAdded {
        c["auto_grant"] = json!(grant);
        ops::add_card(state, &triage(), serde_json::from_value(c).unwrap()).await.unwrap()
    }

    #[tokio::test]
    async fn a_strict_rule_releases_its_card_at_once_and_logs_it() {
        let state = deck().await;
        let g = sectigo_rule(&state).await;
        let added = post(&state, card("work", "orders@mail.sectigo.com", true, forward_to_esign()), g).await;
        let auto = added.auto.unwrap();
        assert!(auto.allow, "{}", auto.reason);
        assert_eq!(added.card.state, CardState::Automatic);
        assert_eq!(added.card.grant_id, Some(g));
        assert_eq!(added.card.action_records[0].status, quietdeck_types::card::ActionStatus::Released);
        let taken: Vec<Released> = decisions::take(&state.db, &triage()).await.unwrap();
        assert_eq!(taken.len(), 1, "released to the posting agent with no undo window");
        let entries = timeline::unread(&state.db, 10).await.unwrap();
        assert_eq!(entries[0].kind, TimelineKind::AutoAction);
        assert_eq!(entries[0].text, format!("Your certificate is issued: forward to esign cert (rule {g})"));
        let rule = crate::store::rules::get(&state.db, g).await.unwrap().unwrap();
        assert_eq!(rule.match_count, 1);
        let inbox = ops::inbox(&state, None).await.unwrap();
        assert!(inbox.actions.is_empty(), "an automatic card isn't waiting for the person");
    }

    #[tokio::test]
    async fn strict_criteria_need_the_sender_and_a_passing_dmarc_or_dkim() {
        let state = deck().await;
        let g = sectigo_rule(&state).await;
        for (c, why) in [
            (card("work", "orders@sectigo.com", false, forward_to_esign()), "DMARC nor DKIM"),
            (card("work", "orders@sectigo.com.evil.example", true, forward_to_esign()), "isn't from sectigo.com"),
            (card("work", "orders@notsectigo.com", true, forward_to_esign()), "isn't from sectigo.com"),
            (card("personal", "orders@sectigo.com", true, forward_to_esign()), "for account work"),
            (
                card(
                    "work",
                    "orders@sectigo.com",
                    true,
                    json!([{"type": "forward", "to": {"node": "server", "tab": "ops"}, "text": "x"}]),
                ),
                "only to esign cert@server",
            ),
            (
                card(
                    "work",
                    "orders@sectigo.com",
                    true,
                    json!([{"type": "archive", "account": "work", "thread_id": "t"}]),
                ),
                "doesn't allow archive",
            ),
        ] {
            let added = post(&state, c, g).await;
            let auto = added.auto.unwrap();
            assert!(!auto.allow);
            assert!(auto.reason.contains(why), "{why}: {}", auto.reason);
            assert_eq!(added.card.state, CardState::Open, "a denied card waits for the person");
        }
    }

    #[tokio::test]
    async fn fuzzy_rules_need_a_known_domain_and_every_project_must_allow_each_action() {
        let state = deck().await;
        let fuzzy: RuleInput = serde_json::from_value(json!({"mode": "fuzzy", "instruction": "CI failure emails",
            "known_sender_domains": ["github.com"],
            "action_template": {"types": ["forward", "archive"], "target": esign()}}))
        .unwrap();
        let g = ops::rule_create(&state, fuzzy).await.unwrap().id;
        let both = json!([{"type": "forward", "to": esign(), "text": "CI failed"},
            {"type": "archive", "account": "personal", "thread_id": "t"}]);
        let ok = post(&state, card("personal", "noreply@github.com", false, both.clone()), g).await.auto.unwrap();
        assert!(ok.allow, "{}", ok.reason);
        let stranger = post(&state, card("personal", "noreply@gitlab.com", false, both), g).await.auto.unwrap();
        assert!(stranger.reason.contains("isn't one of the rule's known sender domains"));
        let work = json!([{"type": "forward", "to": esign(), "text": "x"}, {"type": "archive", "account": "work", "thread_id": "t"}]);
        let r = post(&state, card("work", "noreply@github.com", false, work), g).await.auto.unwrap();
        assert!(
            r.reason.contains("no project in deck.yaml covers account work, so no policy allows archive"),
            "{}",
            r.reason
        );
        let label = json!([{"type": "label", "account": "personal", "thread_id": "t", "add": ["x"]}]);
        let r = post(&state, card("personal", "noreply@github.com", false, label), g).await.auto.unwrap();
        assert!(r.reason.contains("doesn't allow label"), "{}", r.reason);
    }

    #[tokio::test]
    async fn automatic_session_starts_stop_at_the_daily_cap() {
        let state = deck().await;
        let to = json!({"node": "server", "tab": "ops"});
        let rule: RuleInput = serde_json::from_value(json!({"mode": "fuzzy", "instruction": "alerts",
            "known_sender_domains": ["alerts.example"], "action_template": {"types": ["new_session"], "target": to}}))
        .unwrap();
        let g = ops::rule_create(&state, rule).await.unwrap().id;
        let start = |n: i32| {
            let mut c = card(
                "personal",
                "a@alerts.example",
                false,
                json!([{"type": "new_session", "node": "server", "tab": "ops", "cwd": "~/code/ops", "prompt": "look"}]),
            );
            c["thread_key"] = json!(format!("t{n}"));
            c
        };
        for n in 0..3 {
            let r = post(&state, start(n), g).await.auto.unwrap();
            assert!(r.allow, "{}", r.reason);
        }
        let r = post(&state, start(3), g).await.auto.unwrap();
        assert!(r.reason.contains("at most 3 automatic resumes and new sessions a day"), "{}", r.reason);
    }

    #[tokio::test]
    async fn a_closed_session_archives_its_grants_and_a_card_to_its_tab_offers_them() {
        let state = deck().await;
        let g = sectigo_rule(&state).await;
        let rule = crate::store::rules::get(&state.db, g).await.unwrap().unwrap();
        assert_eq!(rule.session_ref.unwrap().session_id.as_deref(), Some("s-esign"), "tied to the running session");
        let at = jiff::Timestamp::now();
        sessions::closed(&state.db, "server", "s-esign", at, Some(&SessionEnd::TabClosed)).await.unwrap();
        ops::session_closed(&state, "server", "s-esign").await.unwrap();
        let rule = crate::store::rules::get(&state.db, g).await.unwrap().unwrap();
        assert_eq!(rule.status, RuleStatus::Archived);
        assert_eq!(rule.archived_reason.as_deref(), Some("its session in esign cert closed"));
        let entries = timeline::unread(&state.db, 10).await.unwrap();
        assert!(entries.iter().any(|e| e.kind == TimelineKind::GrantArchived));
        let denied = post(&state, card("work", "orders@sectigo.com", true, forward_to_esign()), g).await.auto.unwrap();
        assert!(denied.reason.contains("archived"), "{}", denied.reason);

        let inbox = ops::inbox(&state, None).await.unwrap();
        let offer = &inbox.rule_offers[0];
        assert_eq!(offer.rule_ids, vec![g]);
        let new_session =
            SessionRef { node: "server".into(), tab: "esign cert".into(), session_id: Some("s-new".into()) };
        let moved = ops::rule_status(
            &state,
            g,
            RuleStatusChange { status: RuleStatus::Active, move_to: Some(new_session.clone()) },
        )
        .await
        .unwrap();
        assert_eq!(moved.session_ref, Some(new_session.clone()));
        assert_eq!(moved.action_template.target, Some(new_session));
    }

    #[tokio::test]
    async fn a_comment_goes_to_the_session_behind_it_and_waits_in_its_history() {
        let state = deck().await;
        let added = ops::add_card(
            &state,
            &triage(),
            serde_json::from_value(card(
                "work",
                "a@b.example",
                true,
                json!([{"type": "archive", "account": "work", "thread_id": "t"}]),
            ))
            .unwrap(),
        )
        .await
        .unwrap();
        let c = ops::comment_add(
            &state,
            CommentAdd { target: CommentTarget::Card(added.card.id), text: "Next time forward these to ops".into() },
        )
        .await
        .unwrap();
        assert_eq!(c.to, Some(triage()));
        assert_eq!(c.on, "Your certificate is issued");
        assert!(c.picked_up_at.is_none(), "no node is connected, so the message couldn't go");
        let h = ops::history(&state, &triage(), "last").await.unwrap();
        assert_eq!(h.comments.len(), 1);
        assert_eq!(h.comments[0].text, "Next time forward these to ops");
        let again = ops::history(&state, &triage(), "last").await.unwrap();
        assert!(again.comments.is_empty(), "picked up once");
        let deck_entry = timeline::log(&state.db, TimelineKind::SessionStarted, &[], "Started ops").await.unwrap();
        let c = ops::comment_add(
            &state,
            CommentAdd { target: CommentTarget::Timeline(deck_entry.id), text: "why?".into() },
        )
        .await
        .unwrap();
        assert_eq!(c.to, Some(triage()), "the deck's own entries go to the fallback tab");
    }
}
