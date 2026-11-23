//! The transcript viewer's reads: a page of a session's Claude Code transcript, turned into the deck's items. The
//! transcript stays on this machine; the deck relays each page to the browser and keeps nothing. Reads go backwards
//! from the end, or from where the last page started, in chunks, so a transcript of a gigabyte is never read whole.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use quietdeck_types::transcript::{ItemBody, Subagent, TranscriptItem, TranscriptPage, TranscriptQuery};
use regex::Regex;
use serde_json::Value;

use crate::summaries::transcript::{
    COMMAND, COMMAND_ARGS, NOISE_COMMANDS, PEER_BODY, SKIP_ORIGINS, SYSTEM_REMINDER, is_auto_continue, parse_ts,
    tool_line, truthy,
};

/// The first page holds at least this many items, and reaches back to the person's last message.
const FIRST_PAGE_MIN: usize = 30;
/// An earlier page holds about this many.
const PAGE_ITEMS: usize = 80;
/// No page holds more than this many items, or reads more than [`MAX_BYTES`].
const MAX_ITEMS: usize = 500;
const MAX_BYTES: u64 = 16 << 20;
const CHUNK: usize = 1 << 20;

const TEXT_CAP: usize = 20_000;
const THINKING_CAP: usize = 8_000;
const TOOL_CAP: usize = 3_000;

/// Transcript lines with the byte offset each starts at.
type Lines = Vec<(u64, Vec<u8>)>;

/// A page of `session_id`'s transcript (or one of its subagents'), as `query` asks.
pub async fn page(home: PathBuf, session_id: String, query: TranscriptQuery) -> Result<TranscriptPage, String> {
    tokio::task::spawn_blocking(move || read_page(&home, &session_id, &query))
        .await
        .map_err(|e| format!("reading the transcript: {e}"))?
}

fn read_page(home: &Path, session_id: &str, query: &TranscriptQuery) -> Result<TranscriptPage, String> {
    if !safe_id(session_id) {
        return Err("that isn't a session ID".into());
    }
    let main = find_claude_transcript(home, session_id).ok_or_else(|| {
        "no Claude Code transcript for this session on this machine (the viewer reads Claude Code sessions only)"
            .to_string()
    })?;
    let subagents_dir = main.with_extension("").join("subagents");
    let subagents = list_subagents(&subagents_dir);
    let path = match &query.subagent {
        Some(id) if safe_id(id) => subagents_dir.join(format!("agent-{id}.jsonl")),
        Some(_) => return Err("that isn't a subagent ID".into()),
        None => main,
    };
    let mut file = File::open(&path).map_err(|e| format!("opening the transcript: {e}"))?;
    let len = file.metadata().map_err(|e| e.to_string())?.len();
    let started_by: HashMap<&str, &str> =
        subagents.iter().filter_map(|s| Some((s.tool_use_id.as_deref()?, s.id.as_str()))).collect();
    let home_str = home.to_string_lossy().to_string();

    let (records, earlier, end) = match (query.after, query.before) {
        (Some(after), _) => {
            let (records, end) = read_forward(&mut file, after.min(len), len).map_err(|e| e.to_string())?;
            (records, Some(after.min(len)), end)
        }
        (None, before) => {
            let end = match before {
                Some(b) => b.min(len),
                None => last_line_end(&mut file, len).map_err(|e| e.to_string())?,
            };
            let first = before.is_none();
            let mut count = 0;
            let mut person = false;
            let records = read_back(&mut file, end, |line| {
                let items = record_items(line, &home_str);
                count += items.len();
                person |= items.iter().any(|b| matches!(b, ItemBody::Person { .. }));
                let enough = if first { person && count >= FIRST_PAGE_MIN } else { count >= PAGE_ITEMS };
                !enough && count < MAX_ITEMS
            })
            .map_err(|e| e.to_string())?;
            let start = records.first().map(|r| r.0).unwrap_or(end);
            (records, (start > 0).then_some(start), end)
        }
    };

    let mut items = Vec::new();
    for (offset, line) in &records {
        let at = serde_json::from_slice::<Value>(line).ok().and_then(|v| parse_ts(v.get("timestamp")));
        for (part, mut body) in record_items(line, &home_str).into_iter().enumerate() {
            if let ItemBody::Tool { id: Some(id), subagent, .. } = &mut body {
                *subagent = started_by.get(id.as_str()).map(|s| s.to_string());
            }
            items.push(TranscriptItem { offset: *offset, part: part as u32, at, body });
        }
    }
    Ok(TranscriptPage {
        session_id: session_id.to_string(),
        subagent: query.subagent.clone(),
        items: pair_results(merge_compactions(items)),
        earlier,
        end,
        subagents,
    })
}

/// Session and subagent IDs become file names: letters, digits and dashes only.
fn safe_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 100 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn find_claude_transcript(home: &Path, session_id: &str) -> Option<PathBuf> {
    let name = format!("{session_id}.jsonl");
    std::fs::read_dir(home.join(".claude/projects")).ok()?.flatten().map(|d| d.path().join(&name)).find(|p| p.is_file())
}

fn list_subagents(dir: &Path) -> Vec<Subagent> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut found: Vec<(String, Subagent)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let id = name.strip_prefix("agent-")?.strip_suffix(".jsonl")?.to_string();
            let meta: Value = std::fs::read(dir.join(format!("agent-{id}.meta.json")))
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or(Value::Null);
            let text = |k: &str| meta.get(k).and_then(Value::as_str).map(str::to_string);
            let depth = match meta.get("spawnDepth") {
                Some(Value::Number(n)) => n.as_u64().unwrap_or(1) as u32,
                Some(Value::String(s)) => s.parse().unwrap_or(1),
                _ => 1,
            };
            let started = first_timestamp(&e.path()).unwrap_or_default();
            Some((
                started,
                Subagent {
                    id,
                    description: text("description"),
                    agent_type: text("agentType"),
                    depth,
                    tool_use_id: text("toolUseId"),
                },
            ))
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found.into_iter().map(|(_, s)| s).collect()
}

fn first_timestamp(path: &Path) -> Option<String> {
    let mut line = String::new();
    BufReader::new(File::open(path).ok()?).take(1 << 20).read_line(&mut line).ok()?;
    let v: Value = serde_json::from_str(&line).ok()?;
    v.get("timestamp").and_then(Value::as_str).map(str::to_string)
}

/// The end of the last complete line: a record still being written is left for the next read.
fn last_line_end(file: &mut File, len: u64) -> std::io::Result<u64> {
    let mut pos = len;
    let mut buf = vec![0u8; 64 << 10];
    while pos > 0 {
        let n = (pos as usize).min(buf.len());
        pos -= n as u64;
        file.seek(SeekFrom::Start(pos))?;
        file.read_exact(&mut buf[..n])?;
        if let Some(i) = buf[..n].iter().rposition(|b| *b == b'\n') {
            return Ok(pos + i as u64 + 1);
        }
    }
    Ok(0)
}

/// Complete lines from `from` towards `len`, at most [`MAX_BYTES`], with their offsets, and where reading stopped.
fn read_forward(file: &mut File, from: u64, len: u64) -> std::io::Result<(Lines, u64)> {
    file.seek(SeekFrom::Start(from))?;
    let mut reader = BufReader::new(file.take((len - from).min(MAX_BYTES)));
    let mut out = Vec::new();
    let mut pos = from;
    loop {
        let mut line = Vec::new();
        let n = reader.read_until(b'\n', &mut line)?;
        if n == 0 || line.last() != Some(&b'\n') {
            break; // the end, or a record still being written
        }
        line.pop();
        if !line.is_empty() {
            out.push((pos, line));
        }
        pos += n as u64;
    }
    Ok((out, pos))
}

/// Lines that end at or before `end`, newest first, until `more` says stop or [`MAX_BYTES`] are read; returned
/// oldest first, with their offsets.
fn read_back(file: &mut File, end: u64, mut more: impl FnMut(&[u8]) -> bool) -> std::io::Result<Lines> {
    let mut out = Vec::new();
    let mut pos = end;
    // Bytes from `pos` up to the start of the last line handed out, not yet split into lines.
    let mut tail: Vec<u8> = Vec::new();
    let mut read = 0u64;
    'outer: while pos > 0 && read < MAX_BYTES {
        let n = (pos as usize).min(CHUNK);
        pos -= n as u64;
        read += n as u64;
        let mut chunk = vec![0u8; n];
        file.seek(SeekFrom::Start(pos))?;
        file.read_exact(&mut chunk)?;
        chunk.extend_from_slice(&tail);
        tail = chunk;
        // Hand out every complete line in `tail` but the first, which may continue before `pos`.
        while let Some(i) = tail[..tail.len().saturating_sub(1)].iter().rposition(|b| *b == b'\n') {
            let mut line = tail.split_off(i + 1);
            if line.last() == Some(&b'\n') {
                line.pop();
            }
            if !line.is_empty() {
                let offset = pos + i as u64 + 1;
                let go_on = more(&line);
                out.push((offset, line));
                if !go_on {
                    break 'outer;
                }
            }
            tail.truncate(i + 1);
        }
        if pos == 0 && !tail.is_empty() {
            if tail.last() == Some(&b'\n') {
                tail.pop();
            }
            if !tail.is_empty() {
                more(&tail);
                out.push((0, std::mem::take(&mut tail)));
            }
        }
    }
    out.reverse();
    Ok(out)
}

static FROM_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"from-name="([^"]*)""#).unwrap());

fn cap(text: &str, n: usize) -> String {
    match text.char_indices().nth(n) {
        Some((i, _)) => format!("{}…", &text[..i]),
        None => text.to_string(),
    }
}

fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

/// One transcript record as the viewer's items.
fn record_items(line: &[u8], home: &str) -> Vec<ItemBody> {
    let Ok(record) = serde_json::from_slice::<Value>(line) else { return Vec::new() };
    let content = record.get("message").and_then(|m| m.get("content"));
    match record.get("type").and_then(Value::as_str) {
        Some("user") => user_items(&record, content),
        Some("assistant") => assistant_items(content, home),
        Some("system") if record.get("subtype").and_then(Value::as_str) == Some("compact_boundary") => {
            vec![ItemBody::Compaction { summary: None }]
        }
        _ => Vec::new(),
    }
}

fn texts_of(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => {
            let mut texts: Vec<String> = blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(Value::as_str).map(str::to_string))
                .collect();
            if blocks.iter().any(|b| b.get("type").and_then(Value::as_str) == Some("image")) {
                texts.push("[image attached]".into());
            }
            texts.join("\n")
        }
        _ => String::new(),
    }
}

fn user_items(record: &Value, content: Option<&Value>) -> Vec<ItemBody> {
    if record.get("isCompactSummary").is_some_and(truthy) {
        return vec![ItemBody::Compaction { summary: Some(cap(&texts_of(content), TEXT_CAP)) }];
    }
    if let Some(Value::Array(blocks)) = content
        && blocks.iter().any(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
    {
        return blocks
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
            .filter_map(|b| {
                Some(ItemBody::ToolResult {
                    id: b.get("tool_use_id").and_then(Value::as_str)?.to_string(),
                    result: cap(&texts_of(b.get("content")), TOOL_CAP),
                    error: b.get("is_error").is_some_and(truthy),
                })
            })
            .collect();
    }
    let joined = texts_of(content);
    let origin = record.get("origin").and_then(|o| o.get("kind")).and_then(Value::as_str);
    if origin == Some("peer") {
        let from = FROM_NAME.captures(&joined).map(|m| unescape(&m[1]));
        let body = PEER_BODY.captures(&joined).map(|m| m[1].to_owned()).unwrap_or_else(|| joined.clone());
        let body = SYSTEM_REMINDER.replace_all(&body, "").trim().to_owned();
        return if body.is_empty() { Vec::new() } else { vec![ItemBody::Peer { from, text: cap(&body, TEXT_CAP) }] };
    }
    let cleaned = SYSTEM_REMINDER.replace_all(&joined, "");
    let text = cleaned.trim();
    if text.is_empty() {
        return Vec::new();
    }
    if origin.is_some_and(|o| SKIP_ORIGINS.contains(&o))
        || text.starts_with("<task-notification")
        || is_auto_continue(text)
    {
        return vec![ItemBody::WakeUp { text: cap(&strip_tags(text), 600) }];
    }
    if record.get("isMeta").is_some_and(truthy) {
        return Vec::new();
    }
    if text.starts_with("This session is being continued from a previous conversation") {
        return vec![ItemBody::Compaction { summary: Some(cap(text, TEXT_CAP)) }];
    }
    if text.starts_with("[Request interrupted by user") {
        return vec![ItemBody::Interrupted];
    }
    if ["<local-command-", "<artifact-content-authored-by-others"].iter().any(|p| text.starts_with(p)) {
        return Vec::new();
    }
    if let Some(m) = COMMAND.captures(text) {
        let mut name = m[1].trim().to_owned();
        if !name.starts_with('/') {
            name.insert(0, '/');
        }
        if NOISE_COMMANDS.contains(&name.as_str()) {
            return Vec::new();
        }
        let args = COMMAND_ARGS.captures(text).map(|a| a[1].trim().to_owned()).unwrap_or_default();
        return vec![ItemBody::Person { text: format!("{name} {args}").trim().to_owned() }];
    }
    vec![ItemBody::Person { text: cap(text, TEXT_CAP) }]
}

static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"</?[a-z][a-z0-9_-]*[^>]*>").unwrap());

/// A wake-up's text without its XML tags, one line per non-empty line.
fn strip_tags(text: &str) -> String {
    let plain = TAG.replace_all(text, "\n");
    plain.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

fn assistant_items(content: Option<&Value>, home: &str) -> Vec<ItemBody> {
    let Some(Value::Array(blocks)) = content else { return Vec::new() };
    blocks
        .iter()
        .filter_map(|block| match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                let text = block.get("text").and_then(Value::as_str).unwrap_or("").trim();
                (!text.is_empty()).then(|| ItemBody::Agent { text: cap(text, TEXT_CAP) })
            }
            Some("thinking") => {
                let text = block.get("thinking").and_then(Value::as_str).unwrap_or("").trim();
                (!text.is_empty()).then(|| ItemBody::Thinking { text: cap(text, THINKING_CAP) })
            }
            Some("tool_use") => {
                let name = block.get("name").and_then(Value::as_str).unwrap_or("?").to_string();
                let input = block.get("input").cloned().unwrap_or(Value::Null);
                let line = tool_line(&name, &input, home).unwrap_or_else(|| quiet_line(&name, &input, home));
                Some(ItemBody::Tool {
                    id: block.get("id").and_then(Value::as_str).map(str::to_string),
                    line: cap(&line, 300),
                    input: cap(&serde_json::to_string_pretty(&input).unwrap_or_default(), TOOL_CAP),
                    name,
                    result: None,
                    error: false,
                    subagent: None,
                })
            }
            _ => None,
        })
        .collect()
}

/// A line for the tools the summaries leave out (reads and searches).
fn quiet_line(name: &str, input: &Value, home: &str) -> String {
    let field = ["file_path", "pattern", "query", "path"].iter().find_map(|k| input.get(*k).and_then(Value::as_str));
    match field {
        Some(f) => format!("{name} {}", if home.is_empty() { f.to_string() } else { f.replace(home, "~") }),
        None => name.to_string(),
    }
}

/// A compaction boundary followed by its summary is one item.
fn merge_compactions(items: Vec<TranscriptItem>) -> Vec<TranscriptItem> {
    let mut out: Vec<TranscriptItem> = Vec::with_capacity(items.len());
    for item in items {
        if let ItemBody::Compaction { summary: Some(s) } = &item.body
            && let Some(TranscriptItem { body: ItemBody::Compaction { summary: prev @ None }, .. }) = out.last_mut()
        {
            *prev = Some(s.clone());
            continue;
        }
        out.push(item);
    }
    out
}

/// Each result goes on its call when the call is in the page; the rest stay for the viewer to place.
fn pair_results(items: Vec<TranscriptItem>) -> Vec<TranscriptItem> {
    let mut out: Vec<TranscriptItem> = Vec::with_capacity(items.len());
    let mut calls: HashMap<String, usize> = HashMap::new();
    for item in items {
        if let ItemBody::ToolResult { id, result, error } = &item.body
            && let Some(&i) = calls.get(id)
            && let ItemBody::Tool { result: r, error: e, .. } = &mut out[i].body
        {
            *r = Some(result.clone());
            *e = *error;
            continue;
        }
        if let ItemBody::Tool { id: Some(id), .. } = &item.body {
            calls.insert(id.clone(), out.len());
        }
        out.push(item);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    fn line(v: Value) -> String {
        format!("{v}\n")
    }

    fn transcript(home: &Path, session: &str, lines: &[String]) -> PathBuf {
        let dir = home.join(".claude/projects/-w");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{session}.jsonl"));
        let mut f = File::create(&path).unwrap();
        for l in lines {
            f.write_all(l.as_bytes()).unwrap();
        }
        path
    }

    fn person(text: &str, ts: &str) -> String {
        line(json!({"type": "user", "timestamp": ts, "message": {"role": "user", "content": text}}))
    }

    fn agent(text: &str) -> String {
        line(json!({"type": "assistant", "message": {"content": [{"type": "text", "text": text}]}}))
    }

    fn kinds(page: &TranscriptPage) -> Vec<String> {
        page.items
            .iter()
            .map(|i| match &i.body {
                ItemBody::Person { text } => format!("person:{text}"),
                ItemBody::Agent { text } => format!("agent:{text}"),
                ItemBody::Tool { line, result, .. } => format!("tool:{line}:{}", result.as_deref().unwrap_or("-")),
                ItemBody::ToolResult { id, .. } => format!("result:{id}"),
                ItemBody::WakeUp { .. } => "wake".into(),
                ItemBody::Peer { from, text } => format!("peer:{}:{text}", from.as_deref().unwrap_or("?")),
                ItemBody::Compaction { summary } => format!("compaction:{}", summary.is_some()),
                ItemBody::Thinking { .. } => "thinking".into(),
                ItemBody::Interrupted => "interrupted".into(),
            })
            .collect()
    }

    #[test]
    fn the_first_page_reaches_back_to_the_persons_last_message_and_pages_go_back_from_there() {
        let home = tempfile::tempdir().unwrap();
        let mut lines = vec![person("first", "2026-11-23T01:00:00Z")];
        for i in 0..200 {
            lines.push(agent(&format!("a{i}")));
        }
        lines.push(person("fix the build", "2026-11-23T02:00:00Z"));
        for i in 0..40 {
            lines.push(agent(&format!("b{i}")));
        }
        transcript(home.path(), "s1", &lines);
        let first = read_page(home.path(), "s1", &TranscriptQuery::default()).unwrap();
        let k = kinds(&first);
        assert_eq!(k.len(), 41, "the person's message and everything after it");
        assert_eq!(k[0], "person:fix the build");
        assert_eq!(k[40], "agent:b39");
        let earlier =
            read_page(home.path(), "s1", &TranscriptQuery { before: first.earlier, ..Default::default() }).unwrap();
        assert_eq!(earlier.items.len(), PAGE_ITEMS);
        assert_eq!(kinds(&earlier).last().unwrap(), "agent:a199");
        assert_eq!(earlier.end, first.earlier.unwrap());
        // Paging back reaches the start.
        let mut before = earlier.earlier;
        let mut seen = earlier.items.len() + first.items.len();
        while let Some(b) = before {
            let p = read_page(home.path(), "s1", &TranscriptQuery { before: Some(b), ..Default::default() }).unwrap();
            seen += p.items.len();
            if p.earlier.is_none() {
                assert_eq!(kinds(&p)[0], "person:first");
            }
            before = p.earlier;
        }
        assert_eq!(seen, lines.len());
    }

    #[test]
    fn after_reads_only_whats_new_and_leaves_a_record_being_written() {
        let home = tempfile::tempdir().unwrap();
        let path = transcript(home.path(), "s2", &[person("go", "2026-11-23T01:00:00Z"), agent("one")]);
        let first = read_page(home.path(), "s2", &TranscriptQuery::default()).unwrap();
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(agent("two").as_bytes()).unwrap();
        f.write_all(br#"{"type":"assistant","message""#).unwrap();
        let next =
            read_page(home.path(), "s2", &TranscriptQuery { after: Some(first.end), ..Default::default() }).unwrap();
        assert_eq!(kinds(&next), vec!["agent:two"]);
        let again =
            read_page(home.path(), "s2", &TranscriptQuery { after: Some(next.end), ..Default::default() }).unwrap();
        assert!(again.items.is_empty(), "the half-written record waits");
        assert_eq!(again.end, next.end);
    }

    #[test]
    fn records_become_items() {
        let home = tempfile::tempdir().unwrap();
        let lines = vec![
            person("<system-reminder>x</system-reminder>please look", "2026-11-23T01:00:00Z"),
            line(json!({"type": "assistant", "message": {"content": [
                {"type": "thinking", "thinking": "hmm"},
                {"type": "tool_use", "id": "t1", "name": "Read", "input": {"file_path": "/h/code/a.rs"}},
                {"type": "tool_use", "id": "t2", "name": "Agent", "input": {"description": "dig"}}
            ]}})),
            line(json!({"type": "user", "message": {"content": [
                {"type": "tool_result", "tool_use_id": "t1", "content": "fn main() {}"},
                {"type": "tool_result", "tool_use_id": "t0", "content": "older", "is_error": true}
            ]}})),
            line(json!({"type": "user", "origin": {"kind": "task-notification"}, "isMeta": true,
                "message": {"content": "<task-notification><summary>build done</summary></task-notification>"}})),
            line(json!({"type": "user", "origin": {"kind": "peer"}, "isMeta": true, "message": {"content":
                "<cross-session-message from-name=\"planning &amp; co\">\nhello\n</cross-session-message>"}})),
            line(json!({"type": "user", "isMeta": true, "message": {"content": "Caveat: local"}})),
            line(json!({"type": "system", "subtype": "compact_boundary"})),
            line(
                json!({"type": "user", "isCompactSummary": true, "message": {"content": "This session is being continued from a previous conversation…"}}),
            ),
            person("<command-name>/model</command-name>", "2026-11-23T01:01:00Z"),
            person("<command-name>review</command-name><command-args>42</command-args>", "2026-11-23T01:02:00Z"),
            person("[Request interrupted by user]", "2026-11-23T01:03:00Z"),
        ];
        std::fs::create_dir_all(home.path().join(".claude/projects/-w/s3/subagents")).unwrap();
        std::fs::write(
            home.path().join(".claude/projects/-w/s3/subagents/agent-abc.meta.json"),
            r#"{"agentType":"Explore","description":"dig","toolUseId":"t2","spawnDepth":"1"}"#,
        )
        .unwrap();
        std::fs::write(home.path().join(".claude/projects/-w/s3/subagents/agent-abc.jsonl"), agent("found it"))
            .unwrap();
        transcript(home.path(), "s3", &lines);
        let page = read_page(home.path(), "s3", &TranscriptQuery::default()).unwrap();
        assert_eq!(
            kinds(&page),
            vec![
                "person:please look",
                "thinking",
                "tool:Read /h/code/a.rs:fn main() {}",
                "tool:Agent: dig:-",
                "result:t0",
                "wake",
                "peer:planning & co:hello",
                "compaction:true",
                "person:/review 42",
                "interrupted",
            ]
        );
        let ItemBody::Tool { subagent, .. } = &page.items[3].body else { panic!() };
        assert_eq!(subagent.as_deref(), Some("abc"));
        assert_eq!(page.subagents.len(), 1);
        assert_eq!(page.subagents[0].description.as_deref(), Some("dig"));
        let sub = read_page(home.path(), "s3", &TranscriptQuery { subagent: Some("abc".into()), ..Default::default() })
            .unwrap();
        assert_eq!(kinds(&sub), vec!["agent:found it"]);
        assert!(
            read_page(home.path(), "s3", &TranscriptQuery { subagent: Some("../x".into()), ..Default::default() })
                .is_err()
        );
        assert!(read_page(home.path(), "../s3", &TranscriptQuery::default()).is_err());
    }

    #[test]
    fn reading_back_over_chunk_boundaries_keeps_every_line_whole() {
        let home = tempfile::tempdir().unwrap();
        let big = "x".repeat(CHUNK + 10);
        let lines = vec![person("start", "2026-11-23T01:00:00Z"), agent(&big), agent("after")];
        let path = transcript(home.path(), "s4", &lines);
        let mut f = File::open(&path).unwrap();
        let len = f.metadata().unwrap().len();
        let all = read_back(&mut f, len, |_| true).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].0, 0);
        assert_eq!(all[1].1.len() + 1, lines[1].len());
        assert_eq!(all[2].0 as usize, lines[0].len() + lines[1].len());
    }
}
