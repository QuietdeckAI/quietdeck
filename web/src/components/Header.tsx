import { Settings } from "lucide-react";
import { cn } from "@/lib/utils";
import { navigate, usePath } from "@/lib/route";

const LINKS = [
  {
    to: "/",
    label: "Inbox",
    match: (p: string) => p === "/" || p.startsWith("/later"),
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

/** The wordmark at one edge, the screens at the other. */
export function Header() {
  const path = usePath();
  return (
    <header className="border-b border-rule">
      <div className="mx-auto flex h-14 max-w-3xl items-center justify-between gap-2 px-4 sm:gap-4">
        <Link
          to="/"
          className="flex shrink-0 items-center font-display text-xl font-medium tracking-tight text-ink"
        >
          {/* Five screens leave no room for the wordmark on a phone: the app's icon stands in for it. */}
          <img src="/icon.svg" alt="" className="size-7 rounded-md sm:hidden" />
          <span className="max-sm:sr-only">Quietdeck</span>
        </Link>
        <nav className="flex items-center gap-1" aria-label="Screens">
          {LINKS.map((l) => (
            <Link
              key={l.to}
              to={l.to}
              aria-current={l.match(path) ? "page" : undefined}
              aria-label={l.label}
              className={cn(
                "rounded-md px-1.5 py-1.5 text-sm whitespace-nowrap text-neutral transition-colors sm:px-2.5 " +
                  "duration-(--dur-quick) hover:text-ink",
                l.match(path) &&
                  "text-ink underline decoration-accent decoration-2 underline-offset-[6px]",
              )}
            >
              {l.to === "/settings" ? (
                <>
                  <Settings aria-hidden className="size-4 sm:hidden" />
                  <span className="max-sm:sr-only">{l.label}</span>
                </>
              ) : (
                l.label
              )}
            </Link>
          ))}
        </nav>
      </div>
    </header>
  );
}

export function Link({
  to,
  className,
  children,
  ...rest
}: React.ComponentProps<"a"> & { to: string }) {
  return (
    <a
      href={to}
      className={className}
      onClick={(e) => {
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
