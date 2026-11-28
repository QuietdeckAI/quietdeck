//! Transcript searches for "who has this ID?". Raw transcripts never leave the machine: the answer is which sessions
//! mention the text, and when they last did.

use std::path::{Path, PathBuf};

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime};

use jiff::Timestamp;
use quietdeck_types::directory::TranscriptHit;
use quietdeck_types::node::TranscriptSearch;

/// Claude Code's transcripts (`~/.claude/projects/<dir>/<session id>.jsonl`, subagents excluded) and Codex's
/// (`~/.codex/sessions/YYYY/MM/DD/rollout-…-<session id>.jsonl`).
pub fn transcript_files(home: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let projects = home.join(".claude/projects");
    if let Ok(dirs) = std::fs::read_dir(&projects) {
        for dir in dirs.flatten() {
            let Ok(files) = std::fs::read_dir(dir.path()) else { continue };
            for f in files.flatten() {
                let p = f.path();
                if p.extension().is_some_and(|e| e == "jsonl")
                    && let Some(stem) = p.file_stem().and_then(|s| s.to_str())
                {
                    out.push((stem.to_string(), p));
                }
            }
        }
    }
    walk_codex(&home.join(".codex/sessions"), &mut out, 0);
    out
}

fn walk_codex(dir: &Path, out: &mut Vec<(String, PathBuf)>, depth: usize) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk_codex(&p, out, depth + 1);
        } else if p.extension().is_some_and(|x| x == "jsonl")
            && let Some(stem) = p.file_stem().and_then(|s| s.to_str())
            && let Some(id) = codex_session_id(stem)
        {
            out.push((id, p));
        }
    }
}

/// `rollout-2026-11-23T10-00-00-<uuid>` → the uuid.
fn codex_session_id(stem: &str) -> Option<String> {
    let id = stem.get(stem.len().checked_sub(36)?..)?;
    (id.len() == 36 && id.chars().filter(|c| *c == '-').count() == 4).then(|| id.to_string())
}

/// How long Fleetlayer's node gets to answer. It searches for up to 25 seconds before answering with what it found, but
/// the CLI waits 30 for the deck, so a node whose Fleetlayer takes longer than this searches for itself instead.
const FLEETLAYER_SEARCH_WITHIN: Duration = Duration::from_secs(20);

/// The deck's search on this machine: Fleetlayer's (v0.4.0, `node/transcript_search`: redacted, subagents and ended
/// sessions included) when its node answers here, else this machine's own [`search`]. An older Fleetlayer, a refusal or
/// no answer in time falls back to the node's own search.
pub async fn search_here(home: PathBuf, query: String, limit: usize) -> TranscriptSearch {
    match via_fleetlayer(&crate::fleetlayer::node_socket(), &query, limit).await {
        Some(found) => found,
        None => search(home, query, limit).await,
    }
}

async fn via_fleetlayer(socket: &Path, query: &str, limit: usize) -> Option<TranscriptSearch> {
    if query.trim().is_empty() {
        return None;
    }
    let params =
        serde_json::json!({"text": query, "limit": limit, "perSession": 1, "why": "who has this, for the deck"});
    match crate::fleetlayer::node_call(socket, "node/transcript_search", params, FLEETLAYER_SEARCH_WITHIN).await? {
        Ok(answer) => Some(from_fleetlayer(&answer, limit)),
        Err(e) => {
            tracing::debug!(error = %e.message, "Fleetlayer's transcript search didn't answer; searching here");
            None
        }
    }
}

/// Fleetlayer's hits as the deck's: one per session (a subagent's hit counts for its session), with its newest
/// mention. A search that ran out of time is marked as searched back to its oldest hit.
fn from_fleetlayer(answer: &serde_json::Value, limit: usize) -> TranscriptSearch {
    let mut hits: Vec<TranscriptHit> = Vec::new();
    for h in answer["hits"].as_array().into_iter().flatten() {
        let Some(session_id) = h["sessionId"].as_str() else { continue };
        let at = h["timestamp"].as_str().and_then(|t| t.parse::<Timestamp>().ok());
        match hits.iter_mut().find(|x| x.session_id == session_id) {
            Some(x) if at > x.last_mentioned_at => x.last_mentioned_at = at,
            Some(_) => {}
            None => hits.push(TranscriptHit { session_id: session_id.to_string(), last_mentioned_at: at }),
        }
    }
    hits.sort_by_key(|h| std::cmp::Reverse(h.last_mentioned_at));
    hits.truncate(limit);
    let searched_back_to = (answer["partial"].as_bool() == Some(true))
        .then(|| hits.iter().filter_map(|h| h.last_mentioned_at).min().unwrap_or_else(Timestamp::now));
    TranscriptSearch { hits, searched_back_to }
}

/// How long one search may take on this machine. The deck waits a little longer than this.
pub const SEARCH_BUDGET: Duration = Duration::from_secs(6);

/// Files are read this much at a time, so a transcript of a gigabyte or more never sits in memory whole.
const CHUNK: usize = 8 << 20;

/// Search every transcript for `query` (a literal, case-sensitive), newest files first, on several threads, until
/// [`SEARCH_BUDGET`] runs out. The newest mention's `timestamp` field is each hit's time.
pub async fn search(home: PathBuf, query: String, limit: usize) -> TranscriptSearch {
    tokio::task::spawn_blocking(move || search_blocking(&home, &query, limit, SEARCH_BUDGET))
        .await
        .unwrap_or(TranscriptSearch { hits: vec![], searched_back_to: None })
}

fn search_blocking(home: &Path, query: &str, limit: usize, budget: Duration) -> TranscriptSearch {
    if query.trim().is_empty() {
        return TranscriptSearch { hits: vec![], searched_back_to: None };
    }
    let deadline = Instant::now() + budget;
    let mut files: Vec<(String, PathBuf, SystemTime)> = transcript_files(home)
        .into_iter()
        .map(|(id, p)| {
            let modified = std::fs::metadata(&p).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
            (id, p, modified)
        })
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.2));

    let next = AtomicUsize::new(0);
    let found: Mutex<Vec<TranscriptHit>> = Mutex::new(Vec::new());
    let finder = memchr::memmem::Finder::new(query.as_bytes());
    let workers = std::thread::available_parallelism().map_or(2, |n| n.get()).clamp(1, 4);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                let mut buf = vec![0u8; CHUNK + query.len()];
                loop {
                    if Instant::now() >= deadline {
                        return;
                    }
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    let Some((session_id, path, _)) = files.get(i) else { return };
                    let Some(at) = last_mention(path, &finder, &mut buf) else { continue };
                    let mut hits = found.lock().unwrap_or_else(|e| e.into_inner());
                    match hits.iter_mut().find(|h| &h.session_id == session_id) {
                        Some(h) if at > h.last_mentioned_at => h.last_mentioned_at = at,
                        Some(_) => {}
                        None => hits.push(TranscriptHit { session_id: session_id.clone(), last_mentioned_at: at }),
                    }
                }
            });
        }
    });

    // Every file a worker took was searched to the end, so the files not searched are the oldest ones.
    let claimed = next.load(Ordering::SeqCst).min(files.len());
    let searched_back_to = (claimed < files.len())
        .then(|| claimed.checked_sub(1).map(|i| files[i].2).unwrap_or_else(SystemTime::now))
        .and_then(|t| Timestamp::try_from(t).ok());
    let mut hits = found.into_inner().unwrap_or_else(|e| e.into_inner());
    hits.sort_by_key(|h| std::cmp::Reverse(h.last_mentioned_at));
    hits.truncate(limit);
    TranscriptSearch { hits, searched_back_to }
}

/// `Some(time of the last line that mentions it)` when the file mentions it at all; the time is `None` when that
/// line has no readable `timestamp`.
fn last_mention(path: &Path, finder: &memchr::memmem::Finder, buf: &mut [u8]) -> Option<Option<Timestamp>> {
    let mut f = File::open(path).ok()?;
    let keep = finder.needle().len().saturating_sub(1);
    let mut filled = 0usize; // bytes in buf, the first `carried` of them repeated from the last chunk
    let mut base = 0u64; // the file offset of buf[0]
    let mut last: Option<u64> = None;
    loop {
        let n = read_some(&mut f, &mut buf[filled..]).ok()?;
        if n == 0 {
            break;
        }
        filled += n;
        if let Some(pos) = finder.find_iter(&buf[..filled]).last() {
            last = Some(base + pos as u64);
        }
        let carried = keep.min(filled);
        buf.copy_within(filled - carried..filled, 0);
        base += (filled - carried) as u64;
        filled = carried;
    }
    let pos = last?;
    Some(timestamp_around(&mut f, pos))
}

fn read_some(f: &mut File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut total = 0;
    while total < buf.len() {
        match f.read(&mut buf[total..])? {
            0 => break,
            n => total += n,
        }
    }
    Ok(total)
}

/// The `timestamp` of the JSON line holding the byte at `pos`: the line's first `"timestamp":"…"`. In content, quotes
/// are escaped, so that byte sequence is a key. It's the first key in Codex's lines and follows `message` in Claude
/// Code's, so lines are read from their start, a step at a time, until it turns up or the line ends.
fn timestamp_around(f: &mut File, pos: u64) -> Option<Timestamp> {
    const STEP: u64 = 1 << 20;
    const KEY: &[u8] = b"\"timestamp\":\"";
    // The line's start: the byte after the last newline before `pos`.
    let mut line_start = 0;
    let mut end = pos;
    let mut buf = Vec::new();
    while end > 0 {
        let from = end.saturating_sub(STEP);
        f.seek(SeekFrom::Start(from)).ok()?;
        buf.clear();
        f.by_ref().take(end - from).read_to_end(&mut buf).ok()?;
        if let Some(i) = buf.iter().rposition(|b| *b == b'\n') {
            line_start = from + i as u64 + 1;
            break;
        }
        end = from;
    }
    // Forward from there to the key, or the line's end.
    f.seek(SeekFrom::Start(line_start)).ok()?;
    let mut carry: Vec<u8> = Vec::new();
    loop {
        buf.clear();
        buf.extend_from_slice(&carry);
        let n = f.by_ref().take(STEP).read_to_end(&mut buf).ok()?;
        let line_end = buf.iter().position(|b| *b == b'\n').unwrap_or(buf.len());
        if let Some(i) = memchr::memmem::find(&buf[..line_end], KEY) {
            let value_start = i + KEY.len();
            // The value is short: make sure all of it is in the buffer.
            if buf.len() < value_start + 64 {
                f.by_ref().take(64).read_to_end(&mut buf).ok()?;
            }
            let value_end = buf[value_start..].iter().position(|b| *b == b'"')? + value_start;
            return std::str::from_utf8(&buf[value_start..value_end]).ok()?.parse().ok();
        }
        if n == 0 || line_end < buf.len() {
            return None;
        }
        let keep = (KEY.len() - 1).min(buf.len());
        carry = buf[buf.len() - keep..].to_vec();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fleetlayers_hits_become_one_per_session_newest_first() {
        let answer = serde_json::json!({"node": "agents", "searched": 40, "hits": [
            {"sessionId": "s1", "offset": 10, "timestamp": "2026-11-23T10:00:00Z", "excerpt": "sc-1234"},
            {"sessionId": "s2", "agentId": "a1", "offset": 5, "timestamp": "2026-11-23T12:00:00Z", "excerpt": "sc-1234"},
            {"sessionId": "s1", "offset": 90, "timestamp": "2026-11-23T11:00:00Z", "excerpt": "sc-1234"},
            {"sessionId": "s3", "offset": 1, "excerpt": "no time"}
        ]});
        let found = from_fleetlayer(&answer, 50);
        let ids: Vec<_> = found.hits.iter().map(|h| h.session_id.as_str()).collect();
        assert_eq!(ids, ["s2", "s1", "s3"]);
        assert_eq!(found.hits[1].last_mentioned_at, Some("2026-11-23T11:00:00Z".parse().unwrap()));
        assert!(found.searched_back_to.is_none());

        let partial = from_fleetlayer(&serde_json::json!({"hits": answer["hits"], "partial": true}), 50);
        assert_eq!(partial.searched_back_to, Some("2026-11-23T11:00:00Z".parse().unwrap()));
    }

    #[tokio::test]
    async fn without_a_fleetlayer_node_the_search_runs_here() {
        let d = tempfile::tempdir().unwrap();
        assert!(via_fleetlayer(&d.path().join("none.sock"), "sc-1", 5).await.is_none());
    }

    #[test]
    fn finds_the_newest_mention_per_session() {
        let home = tempfile::tempdir().unwrap();
        let proj = home.path().join(".claude/projects/-w");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(
            proj.join("s1.jsonl"),
            "{\"timestamp\":\"2026-11-23T10:00:00Z\",\"text\":\"sc-1234\"}\n{\"timestamp\":\"2026-11-23T11:00:00Z\",\"text\":\"about sc-1234 again\"}\n{\"timestamp\":\"2026-11-23T12:00:00Z\",\"text\":\"other\"}\n",
        )
        .unwrap();
        std::fs::write(proj.join("s2.jsonl"), "{\"timestamp\":\"2026-11-23T10:00:00Z\",\"text\":\"nothing\"}\n")
            .unwrap();
        let codex = home.path().join(".codex/sessions/2026/11/23");
        std::fs::create_dir_all(&codex).unwrap();
        std::fs::write(
            codex.join("rollout-2026-11-23T09-00-00-0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b.jsonl"),
            "{\"timestamp\":\"2026-11-23T09:00:00Z\",\"x\":\"sc-1234\"}\n",
        )
        .unwrap();
        let r = search_blocking(home.path(), "sc-1234", 10, Duration::from_secs(5));
        assert_eq!(r.searched_back_to, None);
        let hits = r.hits;
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].session_id, "s1");
        assert_eq!(hits[0].last_mentioned_at.unwrap().to_string(), "2026-11-23T11:00:00Z");
        assert_eq!(hits[1].session_id, "0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b");
    }

    #[test]
    fn matches_across_chunk_edges_and_long_lines() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.jsonl");
        // The needle straddles the first chunk boundary, on a line far longer than the timestamp window.
        let pad = "x".repeat(CHUNK - 40);
        let tail = "y".repeat(3 << 20);
        std::fs::write(
            &p,
            format!("{{\"timestamp\":\"2026-11-23T08:00:00Z\",\"a\":\"{pad}NEEDLE-123456789{tail}\"}}\n"),
        )
        .unwrap();
        let finder = memchr::memmem::Finder::new(b"NEEDLE-123456789");
        let mut buf = vec![0u8; CHUNK + 16];
        let at = last_mention(&p, &finder, &mut buf).unwrap().unwrap();
        assert_eq!(at.to_string(), "2026-11-23T08:00:00Z");
        assert!(last_mention(&p, &memchr::memmem::Finder::new(b"absent"), &mut buf).is_none());

        // Claude Code's order: the timestamp follows a long message, after an earlier line.
        let msg = "z".repeat(3 << 20);
        std::fs::write(
            &p,
            format!(
                "{{\"timestamp\":\"2026-11-22T01:00:00Z\"}}\n{{\"message\":\"{msg} NEEDLE-123456789 {msg}\",\"uuid\":\"u\",\"timestamp\":\"2026-11-23T09:30:00Z\"}}\n"
            ),
        )
        .unwrap();
        let at = last_mention(&p, &finder, &mut buf).unwrap().unwrap();
        assert_eq!(at.to_string(), "2026-11-23T09:30:00Z");
    }

    #[test]
    fn a_spent_budget_says_how_far_back_it_got() {
        let home = tempfile::tempdir().unwrap();
        let proj = home.path().join(".claude/projects/-w");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(proj.join("s1.jsonl"), "{}\n").unwrap();
        let r = search_blocking(home.path(), "q", 10, Duration::ZERO);
        assert!(r.hits.is_empty());
        assert!(r.searched_back_to.is_some());
    }
}
