# Quietdeck

Clear your inbox in one pass. Your agents do the work.

An AI agent reads every email and writes you one page: what needs a decision at the top, everything that can just be
archived at the bottom. You change what you'd do differently and press Approve All. Work goes to the AI
session that should handle it, and nothing happens to your mail until you approve it. Source-available (Fair Source,
FSL-1.1-ALv2: each release becomes Apache 2.0 after two years), early, built in public:
https://github.com/QuietdeckAI/quietdeck

## Email is where the work starts

A customer asks for something, a build fails, an invoice arrives, a vendor says the certificate is ready. Most of it is
work for an agent, and AI agents are good at that work now. Getting it from your inbox to the right agent is still on
you.

- Gmail's notifications are too noisy, so mail piles up until you get round to it.
- Each email that's really a task means reading it, working out which session should handle it, and pasting it in.
- CI failures and alerts land in your inbox even when an agent is already watching that build.
- You still want to see every receipt, with its amount, before it's archived. Forty emails one at a time takes the
  entire morning.

## One pass, and the work is handed out

- **One page, decisions first.** What needs you is numbered at the top. Receipts, newsletters and pitches are one line
  each under Archive all. Three emails about the same thing become one item, and its mail menu lists every email
  behind it, with a summary and a link to open it in Gmail.
- **Approve All hands out the work.** Each email goes to the AI session that owns it. Approve All resumes a
  session that has ended, or starts a new one in the right folder, and the agents get on with it.
- **Change anything in plain words.** Open a thread under any item and say what you'd do instead. The agent updates
  its proposal and waits for you again.
- **Alerts go to the agent that owns them.** A service's alerts go straight to its agent, and the report gets a count.
  An agent watching CI can keep those emails out of your inbox while it works.

## Your agents' work, on the same kind of page

The Work tab is one more document, kept by an agent that watches your other sessions. At the top is what's waiting on
you: a session stuck on a prompt, a question, a finished session to wrap up, a pull request to review, today's tasks.
Progress and completions go in an FYI list underneath. Every entry has its own thread and proposed actions, and its
session icon shows a summary and the transcript.

Park a project you won't touch this week, or snooze it until Monday. Name your own tabs and split Work into documents,
one for each part of your life. Work: https://quietdeck.ai/work.md

## Real mail out of spam, fake mail out of your inbox

Gmail's spam filter gets it wrong in both directions. The agent reads what lands in spam as well as your inbox, and
checks each email's sender, whether DKIM and SPF pass, and the domain letter by letter: `ledgerIine.com` with a
capital I where the l should be.

Obvious spam stays where it is. A real invoice that landed in spam comes to the report with Mark as not spam as its
default. An email that fails those checks goes to the top as likely phishing, with what gave it away, and nothing in
it is passed to an agent.

## Nothing happens without you seeing it

The agent proposes, and you approve. Plain code carries out what you approved, exactly as you approved it.

- Every archive, label and draft waits on the page until you approve it, and the runner checks the thread hasn't
  changed since the agent looked.
- Nothing is ever sent. A reply is saved as a Gmail draft for you to edit and send.
- Approve All waits a few seconds with an Undo button, and leaves out anything that changed while you were reading.
- Mail you archive in Gmail yourself drops off the report.

By default solo's agent is your own Claude Code with full access, so it can read whole threads, your notes and the web
to work out what each email needs. It reads strangers' mail with that access, and an email written to fool it could act
as you. A restricted mode limits it to Quietdeck's own command. Quietdeck solo and your security:
https://quietdeck.ai/solo-security.md

## Two ways to run it

- **Quietdeck solo, on one computer.** One program on your Mac or Linux machine. It reads Gmail through gog
  (https://github.com/steipete/gogcli), runs Claude Code as the agent, and carries out archives, labels and drafts.
  Your mail never passes through anyone else's server. https://quietdeck.ai/solo.md
- **A deck on your own server, for all your agents.** The full Quietdeck connects agent sessions on every machine you
  use. Approve All forwards work to them, resumes and starts sessions, and alerts, Matrix rooms and pings on your phone
  come in too. It's designed to run on Fleetlayer (https://fleetlayer.ai), but works without it.
  https://quietdeck.ai/getting-started.md

How the parts fit together (cards, the report and its defaults, Approve All and undo, rules, leases):
https://quietdeck.ai/concepts.md

## Start on your own computer

You need a Mac or Linux machine, Gmail, gog and Claude Code. No prebuilt releases yet:

```sh
git clone https://github.com/QuietdeckAI/quietdeck
cd quietdeck && pnpm -C web install && pnpm -C web build
cargo install --locked --path crates/quietdeck-solo
quietdeck-solo init && quietdeck-solo install
quietdeck-solo open
```

The guide: https://quietdeck.ai/solo.md
