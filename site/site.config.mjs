// What the site publishes from docs/, in sidebar order. A doc not listed here (or in `unpublished`) gets a warning
// at build time, so a new doc isn't forgotten.
// Adapted from Fleetlayer's site (Apache 2.0): see NOTICE.
export const repo = "https://github.com/QuietdeckAI/quietdeck";

export const site = {
  url: "https://quietdeck.ai",
  title: "Quietdeck",
  summary:
    "Quietdeck is an inbox for you and your AI agents: an agent triages your email into one short report, drafts replies and proposes what to archive, and nothing happens until you approve it. Start with Quietdeck solo on your own computer, with your Gmail and Claude Code. Source-available (Fair Source, FSL-1.1-ALv2), self-hosted.",
  // Working files that stay in the repo only. An entry ending in "/" leaves out a whole folder.
  unpublished: ["guides-to-write", "drafts/"],
};

// file: path under docs/ without .md. group: the sidebar group. label: a shorter sidebar name (the page keeps its
// own title). description: overrides the first sentence of the page.
export const pages = [
  { file: "solo", group: "Start here", label: "Quietdeck solo" },
  { file: "solo-security", group: "Start here", label: "How solo keeps mail safe" },
  { file: "solo-reference", group: "Start here", label: "solo.yaml and commands" },
  { file: "concepts", group: "Start here", label: "How Quietdeck works" },
  { file: "getting-started", group: "A deck for many machines", label: "Getting started" },
  { file: "secrets-management", group: "A deck for many machines", label: "Secrets for agents" },
  { file: "agent-instructions", group: "A deck for many machines", label: "Instructions for agents" },
  { file: "executor", group: "A deck for many machines", label: "Executor and the runner" },
  { file: "guides/while-you-read", group: "Guides", label: "While you read, nothing lands" },
  { file: "guides/archive-in-gmail", group: "Guides", label: "Archive in Gmail" },
  { file: "guides/headless-agent-one-cli", group: "Guides", label: "Untrusted mail, one command" },
  { file: "guides/localhost-web-app", group: "Guides", label: "Localhost isn't safe by itself" },
  { file: "guides/mail-through-gog", group: "Guides", label: "Gmail through gog" },
  { file: "guides/triage-agent-own-user", group: "Guides", label: "Triage on its own Unix user" },
  { file: "guides/agents-messaging", group: "Guides", label: "Agents messaging each other" },
  { file: "guides/alerts-to-owners", group: "Guides", label: "Alerts to the agent that owns them" },
  { file: "guides/matrix-source", group: "Guides", label: "Matrix messages" },
  { file: "guides/move-a-session", group: "Guides", label: "Move a session to another machine" },
  { file: "leases", group: "Reference", label: "Leases" },
  { file: "orchestrators", group: "Reference", label: "Orchestrators" },
  { file: "claude-code-messaging", group: "Reference", label: "Claude Code messaging" },
  { file: "summaries-interface", group: "Advanced", label: "Summaries interface" },
  { file: "usage-interface", group: "Advanced", label: "Usage interface" },
];
