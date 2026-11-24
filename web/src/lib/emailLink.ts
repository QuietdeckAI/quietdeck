import type { AccountLabel } from "@/types/AccountLabel";
import type { Card } from "@/types/Card";

/**
 * Where "View email" goes: the card's own link when the agent gave one, else the thread in Gmail, opened as the
 * account's address (deck.yaml), so the right mailbox opens whichever account is signed in first. Null without either.
 */
export function emailLink(card: Card, accounts: AccountLabel[]): string | null {
  if (card.link) return card.link;
  const address = accounts.find((a) => a.id === card.account)?.address;
  if (!address) return null;
  // Older cards' keys carry a `gmail:<account>:` prefix; a Gmail thread ID is hex, so it's what follows the last colon.
  const thread = card.thread_key.split(":").at(-1) ?? card.thread_key;
  // `authuser` picks the signed-in account by address; `/mail/u/<address>/` gives Gmail's "Temporary Error (404)".
  return `https://mail.google.com/mail/?authuser=${encodeURIComponent(address)}#all/${encodeURIComponent(thread)}`;
}
