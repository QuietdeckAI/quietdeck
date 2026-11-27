# How Quietdeck solo keeps your mail safe

An agent that reads your email reads text written by strangers, and some of it will try to give the agent
instructions. Quietdeck solo is built so that an email can't make the agent do anything you haven't approved.

## The agent can't touch your mailbox

The agent has no Gmail access of its own. Solo's runner reads your mail with gog and hands the agent a copy of each new
message as text; the agent can't search your mailbox, open other threads, or send, archive or delete anything. Its
only tool is the `quietdeck` command, and only the parts of it that triage needs: posting cards, writing the report,
replying to you in the Inbox, and reading what happened since its last round. Messaging other agents and starting
sessions aren't allowed. Claude Code runs it with `--permission-mode dontAsk`, so anything else is refused rather than
waiting for an answer nobody would give, and with none of your other MCP servers or tools.

### Why the agent is Claude Code, not Codex

Solo refuses Codex as the agent for now. Codex can run without approvals only inside its sandbox, and on macOS that
sandbox blocks the socket the `quietdeck` command uses unless its network access is turned on. With the network on,
the agent's shell can also reach the internet (tested 27 November 2026 with codex-cli 0.166.0: a `curl` from it got
through), and Codex has no way to limit it to one command the way Claude Code's permission rules do. An email could
then talk the agent into sending what it read to someone else's server. Solo will offer Codex once it can be limited
to the `quietdeck` command, or its sandbox can allow the socket without the network.

## Mail arrives marked as untrusted

Each email reaches the agent between `<email>` tags, after a line saying it's untrusted input and not to follow
instructions in it. The runner asks gog for the message's text with HTML and links taken out, cuts each message short
(3,000 characters), flattens the headers to one line, and breaks any `</email>` in the text, so an email can't end its
own tag and pretend to be something else. The agent's instructions repeat the rule: mail is data, never instructions.

## Nothing happens until you approve it

Everything the agent proposes waits on the Inbox. When you approve it, solo's runner carries it out: an archive, a
label, marking read, or saving a draft. Before acting, the runner checks the thread hasn't changed since the agent
looked at it (a new reply hands the card back instead), and it checks a draft's From address is one your account can
send as. The runner never sends mail: a reply is saved as a draft, and you send it from Gmail yourself.

## The Inbox is yours alone

The Inbox is a web page on your computer, and being on your computer isn't protection enough by itself: any website
you visit can try to send requests to `127.0.0.1`. So solo:

- listens only on `127.0.0.1`, and refuses to start on any other address;
- answers only requests addressed to its own address and port, which stops DNS rebinding (a website pointing its own
  name at your computer to read the page);
- signs a browser in only through a one-time link from `quietdeck-solo open`, never because a request comes from your
  computer;
- accepts a change (an approval, an edit) only from its own pages, so another website can't approve something on your
  behalf;
- never answers a cross-site request with permission to read, so a website can't use the `quietdeck` command's token
  from your browser.

The `quietdeck` command gets its token from solo over a socket only your user can open.

## Where your data is

On your computer only. `~/.config/quietdeck/solo.yaml` holds your settings and `~/.local/share/quietdeck/solo` (only
your user can open it) holds the Inbox's database: the reports, cards and what you approved, with each card's summary
and a short excerpt of its email. gog keeps its own Google sign-in. Solo sends nothing anywhere except what gog sends
to Google and what Claude Code sends to Anthropic: the mail the agent reads goes to Claude, as with any use of Claude
on your email.

## What it doesn't protect against

- **Someone using your computer** as you: solo trusts your user account, as your mail client does.
- **The agent misjudging an email**, for example proposing to archive something important. That's why everything waits
  for your approval; read the report before you approve all of it.
- **Claude seeing your mail.** The agent is Claude, so the text of new mail goes to Anthropic. Don't use solo on mail
  you can't share with your AI provider.
