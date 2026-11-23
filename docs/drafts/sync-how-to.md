# Syncing your agents' memory between machines

**This is a hard requirement.** An agent is only as useful as what it knows
about you: the instructions you've given it, the skills you've written, the
memories it saved after you corrected it, the notes it reads, the reviews and
plans it wrote last week. When that lives on one laptop, every agent on any
other machine (a server VM, a second laptop, a cloud sandbox) starts from
nothing, repeats the mistakes you already fixed, and asks the questions you
already answered. With a dozen agents across machines, Quietdeck will be hard
to get value from until this is set up. Set it up first.

You need two things, and they aren't the same:

- **Sync:** every machine sees the same files within seconds or minutes, and
  any machine can change them. Memories get written everywhere, so this has to
  be two-way.
- **Backup with history:** when a sync tool faithfully copies a bad edit or a
  deletion everywhere, you can get yesterday's version back. Sync alone isn't a
  backup.

## What to sync

| Sync                                                     | Where (Claude Code)                                         |
| -------------------------------------------------------- | ----------------------------------------------------------- |
| Your global instructions                                 | `~/.claude/CLAUDE.md` (Codex: `~/.codex/AGENTS.md`)         |
| Your skills, and anything they link to                   | `~/.claude/skills/` (symlinks into `~/.agents/skills/` too) |
| Subagent definitions, hooks, plans                       | `~/.claude/agents/`, `~/.claude/hooks/`, `~/.claude/plans/` |
| Per-project memories                                     | `~/.claude/projects/<project>/memory/` (see below)          |
| Your notes, if agents read or write them                 | an Obsidian vault or any folder of Markdown                 |
| Saved reviews and other artifacts you want agents to see | wherever your skills write them                             |

**Never sync:**

- **Sign-ins:** `~/.claude/.credentials.json`, extra account folders, the
  macOS Keychain, `gh`/`gcloud` tokens. Two machines refreshing the same
  OAuth token race each other and sign each other out. Sign in on each machine.
- **Per-machine state:** `~/.claude.json` (MCP servers, per-project state),
  `settings.local.json`, `sessions/`, `shell-snapshots/`, `ide/`, caches.
- **Live transcripts** (`~/.claude/projects/*/*.jsonl`): large, append-only and
  written constantly. Back them up one way instead. Quietdeck already collects
  each session's summary on the deck.

**Per-project memories are keyed by path.** Claude Code names the folder after
the project's absolute path, so `/Users/you/code/app` on a Mac is
`-Users-you-code-app` and `/home/you/code/app` on Linux is
`-home-you-code-app`. Either keep projects at the same path on every machine,
or sync the memories through one shared folder and symlink each machine's
`memory/` to it.

## The options

| Option                                   | Good for                                                                   | Watch out for                                                                                                                       |
| ---------------------------------------- | -------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| **Syncthing**                            | Any folder, peer to peer, no server or account. Daemons for macOS and Linux | Conflicts become `.sync-conflict-*` files you clean up by hand. Turn on file versioning for some history. Copies symlinks as links |
| **Obsidian Sync** (official)             | Notes vaults, with the phone. Encrypted, versioned, nothing to run         | Paid, with a storage cap. Needs the Obsidian app (or its headless client) on each machine. Built for vaults, not `~/.claude`        |
| **CouchDB + Self-hosted LiveSync**       | Notes vaults you host yourself, with the phone. Encrypted, runs as a daemon | You run CouchDB. Skips hidden files. First sync of a big vault is slow. Two edits to the same lines: newer wins                    |
| **Google Drive** (Drive for desktop, rclone bisync) | Already paid for, works everywhere, version history                  | No official Linux client: rclone bisync runs on a timer, not live. Its macOS client can hold files online-only                     |
| **git** (a private repo, a timer commits and pulls) | Full history, diffs, review                                      | Not live. Merge conflicts stop the timer until someone resolves them. Never commit sign-ins                                        |

Pick by what you already run: Syncthing is the simplest for `~/.claude`;
reuse your notes tool (Obsidian Sync or LiveSync) for the vault. Many people
use both. Whatever you pick, add a backup with history underneath (restic,
Time Machine, a git snapshot on a timer), and check it can restore.

Quietdeck's hosted VMs (planned) will come with a best-practice setup switched
on, which you can remove or switch to Obsidian Sync, LiveSync, Google Drive or
your own.
