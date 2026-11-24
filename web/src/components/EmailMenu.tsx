import { ChevronLeft, ChevronRight, ExternalLink, Mail } from "lucide-react";
import { useRef, useState } from "react";
import { useInfo } from "@/components/AccountTabs";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { emailLink } from "@/lib/emailLink";
import { when } from "@/lib/format";
import type { AccountLabel } from "@/types/AccountLabel";
import type { Card } from "@/types/Card";

const sentAt = (c: Card) => when(c.last_message_at ?? c.prepared_at);

function from(c: Card): string {
  if (!c.sender) return "Unknown sender";
  return c.sender.name ? `${c.sender.name} <${c.sender.address}>` : c.sender.address;
}

/** Which of the person's addresses it went to: the card's own `to`, else its account's address. */
function to(c: Card, accounts: AccountLabel[]): string {
  const account = accounts.find((a) => a.id === c.account);
  return c.to ?? account?.address ?? account?.label ?? c.account;
}

function ViewEmail({ card, accounts }: { card: Card; accounts: AccountLabel[] }) {
  const href = emailLink(card, accounts);
  if (href === null)
    return (
      <DropdownMenuItem disabled>
        Open in Gmail <ExternalLink />
      </DropdownMenuItem>
    );
  return (
    <DropdownMenuItem asChild>
      <a href={href} target="_blank" rel="noreferrer noopener">
        Open in Gmail <ExternalLink />
      </a>
    </DropdownMenuItem>
  );
}

/**
 * The email's details at the bottom of the menu: the subject, when, exactly who sent it and to which address, then
 * the first lines of what it says.
 */
function Details({ card, accounts }: { card: Card; accounts: AccountLabel[] }) {
  const rows: [string, string][] = [
    ["Date", sentAt(card)],
    ["From", from(card)],
    ["To", to(card, accounts)],
  ];
  return (
    <div className="px-2 pt-1 pb-1.5 text-xs">
      <p className="mb-1.5 text-sm text-ink [overflow-wrap:anywhere]">{card.subject}</p>
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-2.5 gap-y-0.5">
        {rows.map(([label, value]) => (
          <div key={label} className="contents">
            <dt className="text-neutral">{label}</dt>
            <dd className="text-muted [overflow-wrap:anywhere]">{value}</dd>
          </div>
        ))}
      </dl>
      {card.excerpt && (
        <p className="mt-2 line-clamp-8 border-t border-rule pt-2 whitespace-pre-line text-ink [overflow-wrap:anywhere]">
          {card.excerpt.trim()}
        </p>
      )}
    </div>
  );
}

/**
 * The envelope beside Change: the email behind the default. For one email: Open in Gmail, then its details. For several: each email by its subject, with
 * when and who sent it under it; the row opens it in Gmail, and its › opens its details, with a way back.
 */
export function EmailMenu({ about, cards }: { about: string; cards: Card[] }) {
  const accounts = useInfo().data?.accounts ?? [];
  const [shown, setShown] = useState<Card | null>(null);
  // The side the menu opened on, kept while it switches between the list and an email's details: otherwise a shorter
  // pane that fits below the button jumps there.
  const [side, setSide] = useState<"top" | "bottom" | undefined>();
  const content = useRef<HTMLDivElement>(null);
  const keepSide = () => {
    const opened = content.current?.dataset.side;
    if (opened === "top" || opened === "bottom") setSide(opened);
  };
  const one = cards.length === 1 ? cards[0] : undefined;
  const detail = one ?? shown;
  // Not modal: a modal menu locks the page's scrolling, which hides the sticky chat bar while it's open.
  return (
    <DropdownMenu modal={false} onOpenChange={(open) => {
        if (!open) {
          setShown(null);
          setSide(undefined);
        }
      }}
    >
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          aria-label={cards.length > 1 ? `The emails for: ${about}` : `The email for: ${about}`}
          title={cards.length > 1 ? "The emails" : "The email"}
          className={
            "inline-flex size-7 shrink-0 items-center justify-center rounded-md text-muted " +
            "transition-[background-color,color,transform] duration-(--dur-quick) ease-(--ease-out) " +
            "hover:bg-paper-3 hover:text-ink active:translate-y-px " +
            "data-[state=open]:bg-paper-3 data-[state=open]:text-ink [&_svg]:size-4"
          }
        >
          <Mail />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        ref={content}
        side={side ?? "bottom"}
        avoidCollisions={side === undefined}
        className="max-h-[70vh] w-80 max-w-[calc(100vw-2rem)] overflow-y-auto"
      >
        {cards.length === 0 && <DropdownMenuItem disabled>No email for this</DropdownMenuItem>}
        {detail && (
          <>
            {!one && (
              <DropdownMenuItem onSelect={(e) => {
                  e.preventDefault();
                  keepSide();
                  setShown(null);
                }}>
                <ChevronLeft /> All emails
              </DropdownMenuItem>
            )}
            <ViewEmail card={detail} accounts={accounts} />
            <DropdownMenuSeparator />
            <Details card={detail} accounts={accounts} />
          </>
        )}
        {!detail && cards.length > 1 && (
          <>
            <DropdownMenuLabel>Open in Gmail</DropdownMenuLabel>
            {cards.map((c) => {
              const href = emailLink(c, accounts);
              const text = (
                <span className="flex min-w-0 flex-col">
                  <span className="truncate">{c.subject}</span>
                  <span className="truncate text-xs text-neutral">
                    {sentAt(c)} · {c.sender?.name ?? c.sender?.address ?? "Unknown sender"}
                  </span>
                </span>
              );
              return (
                <div key={c.id} className="flex items-start gap-0.5">
                  {href === null ? (
                    <DropdownMenuItem disabled className="min-w-0 flex-1">
                      {text}
                    </DropdownMenuItem>
                  ) : (
                    <DropdownMenuItem asChild className="min-w-0 flex-1">
                      <a href={href} target="_blank" rel="noreferrer noopener">
                        {text}
                      </a>
                    </DropdownMenuItem>
                  )}
                  <DropdownMenuItem
                    aria-label={`Details: ${c.subject}`}
                    className="mt-0.5 size-7 shrink-0 justify-center px-0"
                    onSelect={(e) => {
                      e.preventDefault();
                      keepSide();
                      setShown(c);
                    }}
                  >
                    <ChevronRight />
                  </DropdownMenuItem>
                </div>
              );
            })}
          </>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
