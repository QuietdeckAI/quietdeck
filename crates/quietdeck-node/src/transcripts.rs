//! Transcript searches for "who has this ID?". Raw transcripts never leave the machine: the answer is which sessions
//! mention the text, and when they last did.

use std::path::{Path, PathBuf};

use jiff::Timestamp;
use quietdeck_types::directory::TranscriptHit;

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

/// Search every transcript for `query` (a literal, case-sensitive). The newest mention's `timestamp` field is the
/// answer's time. Runs on a blocking thread.
pub async fn search(home: PathBuf, query: String, limit: usize) -> Vec<TranscriptHit> {
    tokio::task::spawn_blocking(move || search_blocking(&home, &query, limit)).await.unwrap_or_default()
}

fn search_blocking(home: &Path, query: &str, limit: usize) -> Vec<TranscriptHit> {
    if query.trim().is_empty() {
        return vec![];
    }
    let finder = memchr::memmem::FinderRev::new(query.as_bytes());
    let mut hits: Vec<TranscriptHit> = Vec::new();
    for (session_id, path) in transcript_files(home) {
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let Some(last) = finder.rfind(&bytes) else { continue };
        let line_start = bytes[..last].iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
        let line_end = bytes[last..].iter().position(|b| *b == b'\n').map_or(bytes.len(), |i| last + i);
        let at = serde_json::from_slice::<serde_json::Value>(&bytes[line_start..line_end])
            .ok()
            .and_then(|v| v.get("timestamp").and_then(|t| t.as_str()).and_then(|t| t.parse::<Timestamp>().ok()));
        match hits.iter_mut().find(|h| h.session_id == session_id) {
            Some(h) if at > h.last_mentioned_at => h.last_mentioned_at = at,
            Some(_) => {}
            None => hits.push(TranscriptHit { session_id, last_mentioned_at: at }),
        }
    }
    hits.sort_by(|a, b| b.last_mentioned_at.cmp(&a.last_mentioned_at));
    hits.truncate(limit);
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

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
        std::fs::write(proj.join("s2.jsonl"), "{\"timestamp\":\"2026-11-23T10:00:00Z\",\"text\":\"nothing\"}\n").unwrap();
        let codex = home.path().join(".codex/sessions/2026/11/23");
        std::fs::create_dir_all(&codex).unwrap();
        std::fs::write(
            codex.join("rollout-2026-11-23T09-00-00-0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b.jsonl"),
            "{\"timestamp\":\"2026-11-23T09:00:00Z\",\"x\":\"sc-1234\"}\n",
        )
        .unwrap();
        let hits = search_blocking(home.path(), "sc-1234", 10);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].session_id, "s1");
        assert_eq!(hits[0].last_mentioned_at.unwrap().to_string(), "2026-11-23T11:00:00Z");
        assert_eq!(hits[1].session_id, "0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b");
    }
}
