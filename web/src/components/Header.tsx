import { cn } from "@/lib/utils";
import { navigate, usePath } from "@/lib/route";

const LINKS = [
  { to: "/", label: "Inbox", match: (p: string) => p === "/" || p.startsWith("/later") },
  { to: "/history", label: "History", match: (p: string) => p.startsWith("/history") },
  { to: "/settings", label: "Settings", match: (p: string) => p.startsWith("/settings") },
];

/** The wordmark at one edge, the three screens at the other. */
export function Header() {
  const path = usePath();
  return (
    <header className="border-b border-rule">
      <div className="mx-auto flex h-14 max-w-3xl items-center justify-between gap-4 px-4">
        <Link to="/" className="font-display text-xl font-medium tracking-tight text-ink">
          Quietdeck
        </Link>
        <nav className="flex items-center gap-1" aria-label="Screens">
          {LINKS.map((l) => (
            <Link
              key={l.to}
              to={l.to}
              aria-current={l.match(path) ? "page" : undefined}
              className={cn(
                "rounded-md px-2.5 py-1.5 text-sm whitespace-nowrap text-neutral transition-colors " +
                  "duration-(--dur-quick) hover:text-ink",
                l.match(path) && "text-ink underline decoration-accent decoration-2 underline-offset-[6px]",
              )}
            >
              {l.label}
            </Link>
          ))}
        </nav>
      </div>
    </header>
  );
}

export function Link({ to, className, children, ...rest }: React.ComponentProps<"a"> & { to: string }) {
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
