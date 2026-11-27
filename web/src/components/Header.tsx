import { Settings } from "lucide-react";
import { useQueryClient } from "@tanstack/react-query";
import { cn } from "@/lib/utils";
import { useMailBadges } from "@/screens/Mail";
import type { MailBadges } from "@/types/MailBadges";
import { navigate, usePath } from "@/lib/route";
import { topTabs, useDocuments } from "@/lib/documents";

const LINKS = [
  {
    to: "/",
    label: "Inbox",
    match: (p: string) => p === "/",
  },
  {
    to: "/drafts",
    label: "Drafts",
    match: (p: string) => p.startsWith("/drafts"),
  },
  {
    to: "/sent",
    label: "Sent",
    match: (p: string) => p.startsWith("/sent"),
  },
  {
    to: "/threads",
    label: "Threads",
    match: (p: string) => p.startsWith("/threads") || p.startsWith("/cards") || p.startsWith("/later"),
  },
  {
    to: "/sessions",
    label: "Sessions",
    match: (p: string) => p.startsWith("/sessions"),
  },
  {
    to: "/history",
    label: "History",
    match: (p: string) => p.startsWith("/history"),
  },
  {
    to: "/rules",
    label: "Rules",
    match: (p: string) => p.startsWith("/rules"),
  },
  {
    to: "/settings",
    label: "Settings",
    match: (p: string) => p.startsWith("/settings"),
  },
];

const underline = "underline decoration-accent decoration-2 underline-offset-[6px]";

/** The wordmark at one edge, the screens at the other. */
export function Header() {
  const path = usePath();
  const badges = useMailBadges().data;
  const docs = useDocuments().data;
  // The documents' own top-level tabs (Work...) go right after the Inbox; the Inbox's tab stays the first link. A
  // renamed Inbox tab keeps its name there.
  const tabs = docs ? topTabs(docs) : [];
  const inboxTab = tabs.find((t) => t.path === "/");
  const links = [
    { ...LINKS[0], label: inboxTab?.name ?? LINKS[0].label },
    ...tabs
      .filter((t) => t.path !== "/")
      .map((t) => ({ to: t.path, label: t.name, match: (p: string) => p === t.path || p.startsWith(`${t.path}/`) })),
    ...LINKS.slice(1),
  ];
  const qc = useQueryClient();
  const clearBadge = (k: "sent") =>
    qc.setQueryData<MailBadges>(["mail-badges"], (b) => (b ? { ...b, [k]: 0 } : b));
  return (
    // Stays at the top while the page scrolls.
    <header className="sticky top-0 z-20 border-b border-rule bg-paper/95 backdrop-blur-sm">
      <div className="mx-auto flex h-14 max-w-3xl items-center justify-between gap-2 px-4 sm:gap-4">
        <Link
          to="/"
          className="flex shrink-0 items-center font-display text-xl font-medium tracking-tight text-ink"
        >
          {/* Six screens leave no room for the wordmark on a phone: the app's icon stands in for it, and below 360px
              not even that (Inbox goes home too). */}
          <img src="/icon.svg" alt="" className="size-7 rounded-md max-[359px]:hidden sm:hidden" />
          <span className="max-sm:sr-only">Quietdeck</span>
        </Link>
        {/* Eight screens don't fit a phone's width: the row scrolls sideways there, with no scrollbar. */}
        <nav
          className="flex min-w-0 items-center gap-1 overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
          aria-label="Screens"
        >
          {links.map((l) => {
            const count = l.to === "/" ? badges?.inbox : l.to === "/drafts" ? badges?.drafts : l.to === "/sent" ? badges?.sent : 0;
            return (
              <Link
                key={l.to}
                to={l.to}
                aria-label={count ? `${l.label}, ${count} new` : l.label}
                aria-current={l.match(path) ? "page" : undefined}
                onClick={() => {
                  // Sent's count of new mail goes the moment it's opened, not when the deck answers. (Drafts counts
                  // the drafts there are, so it stays.)
                  if (l.to === "/sent") clearBadge("sent");
                }}
                className={cn(
                  "inline-flex items-center gap-1 rounded-md px-1.5 py-1.5 text-sm whitespace-nowrap text-neutral transition-colors max-[359px]:px-1 sm:px-2.5 " +
                    "duration-(--dur-quick) hover:text-ink",
                  l.match(path) && "text-ink",
                )}
              >
                {/* The current screen's line is under its name only, not its count. */}
                {l.to === "/settings" ? (
                  <>
                    <Settings aria-hidden className="size-4 sm:hidden" />
                    <span className={cn("max-sm:sr-only", l.match(path) && underline)}>{l.label}</span>
                  </>
                ) : (
                  <span className={cn(l.match(path) && underline)}>{l.label}</span>
                )}
                {!!count && (
                  <span className="min-w-4 rounded-full bg-paper-3 px-1 text-center text-[11px] leading-4 tabular-nums text-muted">
                    {count}
                  </span>
                )}
              </Link>
            );
          })}
        </nav>
      </div>
    </header>
  );
}

export function Link({
  to,
  className,
  children,
  onClick,
  ...rest
}: React.ComponentProps<"a"> & { to: string }) {
  return (
    <a
      href={to}
      className={className}
      onClick={(e) => {
        onClick?.(e);
        if (e.metaKey || e.ctrlKey || e.shiftKey || e.button !== 0) return;
        e.preventDefault();
        navigate(to);
      }}
      {...rest}
    >
      {children}
    </a>
  );
}
