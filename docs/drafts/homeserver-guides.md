# homeserver's drafts for Quietdeck's guides

Drafts for the quietdeck tab to edit into `docs/`. Placeholders: `<vm>` (the
server's tailnet name), `<user>` (the login user on it), `<tailnet>` (your
tailnet's `*.ts.net` name), `<agents-vault>`, `<hooks.example.com>`.

---

## The triage tab on a server VM

Triage runs on a machine that's always on. Your laptop sleeps, travels and
reboots for updates; mail keeps arriving. On an always-on machine the triage
agent is one long-lived Claude Code session that sits idle almost all the time
and wakes when the runner tells it new mail has arrived, so a card is waiting
for you whenever you open Quietdeck, from any device. Until it's set up, use
the email-triage skill in a chat on your laptop. That stays the fallback
afterwards too.

### The machine

Anything that's always on and runs Linux: a VM on a home server, a small cloud
server, a spare mini PC. Two cores, 4 to 8 GB of memory and 40 GB of disk is
plenty for the triage tab and a few more sessions. Give it:

- **A login user without sudo** for your agents. Install software as an admin,
  through whatever manages the machine, so an agent can't change the system.
- **Lingering**, so that user's services run with nobody logged in:
  `sudo loginctl enable-linger <user>`.
- **Tailscale**, with Tailscale SSH and a tag for the machine. In your tailnet
  policy, let your own devices SSH in as `<user>` with `"action": "accept"`.
  The default `check` asks you to sign in again in a browser every so often,
  and herdr, which keeps an SSH connection open and reconnects after every
  sleep, can't answer that. Let the tag reach your deck on port 443 and nothing
  else it doesn't need.
- **Claude Code**, signed in on that machine (`claude auth login` in a terminal
  there). Never copy another machine's account folders: two machines refreshing
  the same sign-in race each other and sign each other out.
- **Your memories, skills and global instructions**, synced (see Syncing
  memories, skills and notes). The email-triage skill must be there.
- **A secrets manager** (see Secrets management for agents), and **Executor**
  or `gog` for Gmail (see Executor).

### herdr on the server, opened from your laptop

Run herdr's server as a user service on the VM, so its tabs keep running when
no laptop is attached:

```ini
# ~/.config/systemd/user/herdr.service
[Unit]
Description=herdr server

[Service]
ExecStart=/usr/local/bin/herdr server
Restart=on-failure

[Install]
WantedBy=default.target
```

`systemctl --user enable --now herdr`. Build or install the same herdr version
your laptop runs: your laptop's herdr attaches to this server, and an older
server lacks what a newer client asks for.

On the laptop, save the machine once:

```bash
herdr machine add <user>@<vm> --label <vm>
```

Its tabs then show in your laptop's herdr next to your local ones. Open the
machine, make a tab named `email triage`, start `claude` in it in the working
directory you want, and give it the email-triage skill's Quietdeck mode. Check
that you can type in it and scroll back from the laptop.

### The runner

Install the Quietdeck node on the VM (Getting started) and switch its runner on
for the `email triage` tab in `node.yaml`. The runner watches Gmail through
Executor with its own token, wakes the tab when mail arrives, and carries out
what you approve. The node also writes that machine's session summaries to the
deck.

---

## Webhooks: the Tunnel for `/hooks/*`

The deck's webhook listener is a separate port that serves nothing but
`POST /hooks/<id>/<secret>`. Webhook senders are on the public internet, and
the deck's app must never be, so put only that path behind a Cloudflare
Tunnel:

1. Create a tunnel managed in Cloudflare (dashboard or Terraform
   `cloudflare_zero_trust_tunnel_cloudflared` with `config_src = "cloudflare"`),
   and a proxied CNAME `<hooks.example.com>` to `<tunnel id>.cfargotunnel.com`.
2. Give it exactly two ingress rules, in this order:

   ```yaml
   ingress:
     - hostname: <hooks.example.com>
       path: ^/hooks/
       service: http://<deck hooks service>:8081
     - service: http_status:404
   ```

   Everything else, on any hostname, gets a 404 from Cloudflare's edge and never
   reaches your network.
3. Run `cloudflared tunnel --no-autoupdate run` next to the deck, with the
   tunnel's token in `TUNNEL_TOKEN` from your secrets manager. It only connects
   out; nothing listens publicly.
4. Don't put Cloudflare Access in front of `/hooks/*` (or add a Bypass for
   it): senders can't sign in, and each endpoint's URL carries its own secret.
5. Set `QUIETDECK_HOOKS_PUBLIC_URL=https://<hooks.example.com>` on the deck so
   it prints endpoint URLs you can paste into senders.

Check it: `curl -i https://<hooks.example.com>/` and any other path answer 404;
the deck's app stays reachable only on your tailnet.

---

## Secrets management: the worked example (a server VM with 1Password)

One setup that works, on the machine from "The triage tab on a server VM":

1. **A vault for agents**, `<agents-vault>`, holding only what agents need.
2. **A service account per machine**, with read and write on that vault and
   nothing else:

   ```bash
   op service-account create "<vm>" --vault <agents-vault>:read_items,write_items --raw
   ```

   It prints the token once. Store it in your own (admin) vault straight away;
   that copy is what your provisioning tool installs on the machine. If the
   machine is lost, revoke that one service account. The others keep working.
3. **The token on the machine**, readable only by the agents' user:

   ```bash
   install -d -m 700 ~/.config/op
   install -m 600 /dev/stdin ~/.config/op/service-account-token   # paste or pipe the token
   ```

   and in `~/.bashrc`, so every terminal, herdr tab and Claude Code session has it:

   ```bash
   if [ -r "$HOME/.config/op/service-account-token" ]; then
     OP_SERVICE_ACCOUNT_TOKEN="$(cat "$HOME/.config/op/service-account-token")"
     export OP_SERVICE_ACCOUNT_TOKEN
   fi
   ```

   Don't sign a bare `op` in on that machine: the service account is the only
   way in.
4. **Everything else comes from the vault.** The machine reads the rest with its
   own token, for example the runner's Executor token, made on Executor's
   Account > Tokens page and stored as item `executor-<vm>`:

   ```bash
   umask 077
   op read "op://<agents-vault>/executor-<vm>/runner_token" > ~/.config/quietdeck/executor-token
   ```

   `node.yaml` names that file (`runner.executor.token_file`). Nothing secret is
   in `node.yaml` itself.

Your own laptop can work the same way: a service account for the tools that
run there unattended, so they never stop to ask for your fingerprint, while a
bare `op` still goes through your own sign-in.
