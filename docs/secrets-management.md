# Secrets management for agents

Every Quietdeck setup needs a secrets manager. Agents need credentials: CLI tokens, API keys, the runner's Executor
token, each machine's node token. Without a secrets manager those end up pasted into chat, config files and
transcripts. Transcripts are kept, synced between machines and summarised, so a secret pasted once spreads to all of
those places.

The pattern this guide sets up, whichever product you use:

- **A vault just for your agents**, holding only what they need. Your own passwords stay elsewhere.
- **A scoped token on each machine** (a service account, machine identity or access token) that can read that vault
  and nothing else. Each machine gets its own, so a token that leaks can be revoked without touching the others.
- **Agents read secrets through the manager's CLI** at the moment they need one, into a variable or one process's
  environment. Nobody pastes a secret anywhere.

## The main example: 1Password

1. **Make a vault for agents**, for example `agents`. Put in it only what your agents need. Agents can add and update
   their own items there, so a token an agent creates goes straight into the vault.
2. **Make a service account** (on 1Password.com, under Developer, then Service Accounts) with access to that vault
   only.
   Give agents read and write access if they should store new items, read only otherwise. Make one per machine, named
   after the machine.
3. **Store its token on the machine** in a file only you can read, and export it in the agent's environment:

   ```sh
   mkdir -p ~/.config/op-agents && chmod 700 ~/.config/op-agents
   # Paste the service account token when prompted; it isn't echoed or kept in your shell history.
   (umask 077; printf 'Token: '; read -rs t && printf '%s\n' "$t" > ~/.config/op-agents/token); echo
   ```

   Then, in your shell's profile (`~/.zshrc`, `~/.bashrc`):

   ```sh
   export OP_SERVICE_ACCOUNT_TOKEN="$(cat ~/.config/op-agents/token)"
   ```

   Claude Code and Codex sessions started from that shell inherit it.
4. **Agents read a secret** with `op read`, or run a command with `op run`, which puts secrets into that one process's
   environment:

   ```sh
   TOKEN="$(op read "op://agents/tracker-api/token")"
   op run --env-file=tracker.env -- ./sync-stories
   ```

   where `tracker.env` holds references, never values: `TRACKER_TOKEN=op://agents/tracker-api/token`.

Keep vault and item names simple, in kebab-case: an apostrophe or a colon in a name breaks `op://` references.
Service accounts are rate-limited, so an agent should read a secret once per task, not once per command.

**If you also use 1Password yourself**, a bare `op` command signs in as you, through the desktop app, and asks for
your Touch ID or password. With `OP_SERVICE_ACCOUNT_TOKEN` set, `op` uses the service account instead and never
reaches your own vaults. That split is the point: agents get their vault, and only you get yours.

## Other options

| Option | What it is | Fits when |
|---|---|---|
| **Infisical** | MIT licensed, self-hostable, with machine identities and `infisical run` | You want it open source and on your own server |
| **Bitwarden Secrets Manager** | Bitwarden's product for machine secrets: machine accounts with access tokens and the `bws` CLI | You already use Bitwarden |
| **Doppler** | Hosted, with service tokens and `doppler run` | You want a hosted service and no server to run |
| **HashiCorp Vault and OpenBao** | Vault (Business Source Licence since 2023) and its open-source fork OpenBao (MPL-2.0), with AppRole or cloud sign-in for machines | You already run one, or want short-lived generated credentials |
| **Your cloud's secret manager** | AWS Secrets Manager, Google Secret Manager, Azure Key Vault, read with the VM's own cloud identity, so no token sits on disk | Your agents run in that cloud |
| **sops with age** | Encrypted files committed to git, decrypted with an age key on each machine | You want secrets as files in git. The age key is then the one secret to guard |

Each has the same shape as the 1Password example: a scoped identity per machine, and a CLI that reads one secret or
runs one command with secrets in its environment (`infisical run`, `bws run`, `doppler run`, `vault kv get`,
`sops exec-env`).

## Rules for your agents

Add these to your global agent instructions (`~/.claude/CLAUDE.md` for Claude Code, `~/.codex/AGENTS.md` for Codex),
adjusted to your secrets manager:

```markdown
# Secrets

Credentials live in the `agents` vault in 1Password, reached through the service account in
`OP_SERVICE_ACCOUNT_TOKEN`. Read one with `op read "op://agents/<item>/<field>"` into a variable, or run a command
with `op run`. Store a new credential there yourself (`op item create --vault agents ...`) rather than in a file.

- Never print a secret: no `echo`, `cat` or `env` that shows one. Transcripts are kept, synced and summarised, so a
  printed secret spreads.
- Never put a secret in a ticket, commit, pull request, chat message, card, forward or ping.
- Never paste a secret into config. Config holds a reference (`op://...`) or a file path.
- Read a secret once per task, not once per command: the service account is rate-limited.
- If a secret leaks, revoke it where it was issued, store the new one in the vault, and tell me.
```

The last rule has a common variation: a token that reached only a model's transcript (printed by mistake in a
session, and nowhere else) can be redacted from that transcript instead of rotated. Whether that's acceptable depends
on where your transcripts go, so choose deliberately, and say which you chose in the snippet.

## How the node reads its token

Neither the node nor the runner has a token in its code or its config file. `node.yaml` names, for each token, either
a file or an environment variable, and your secrets manager fills it.

**A file**, the default. `quietdeck node enroll` writes the node's token to `~/.config/quietdeck/node-token` with mode
600, in a directory with mode 700, and `node.yaml` points at it:

```yaml
node:
  name: <machine>
  deck: https://<deck address>
  token_file: ~/.config/quietdeck/node-token
```

The node refuses to start if other users can read the file. To rebuild a machine without enrolling it again, keep a
copy of the token in your vault and write the file from it:

```sh
(umask 077; op read "op://agents/quietdeck-node-<machine>/token" > ~/.config/quietdeck/node-token)
```

**An environment variable.** Name it with `token_env` instead of `token_file`, and start the node under your secrets
manager, so the token exists only in the node's own environment:

```yaml
node:
  name: <machine>
  deck: https://<deck address>
  token_env: QUIETDECK_NODE_TOKEN
```

```sh
QUIETDECK_NODE_TOKEN="op://agents/quietdeck-node-<machine>/token" op run -- quietdeck-node
```

(or `infisical run`, `doppler run` and so on). The service that `quietdeck-node install` sets up uses `token_file`; if
you use `token_env`, run the node from your own service definition under `op run`.

The runner's Executor token works the same way, under `runner.executor` with its own `token_file` or `token_env`
([Executor, and the runner that uses it](executor.md#1-a-token-for-the-runner)).

Either way, the node never sends its token anywhere but the deck's sign-in, and never logs it. The CLI never sees it:
it gets a short-lived session token from the node over a local socket only you can open, and that token names the
session that asked for it.
