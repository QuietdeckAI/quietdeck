import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { setParam, useSearch } from "@/lib/route";
import { cn } from "@/lib/utils";
import type { DeckInfo } from "@/types/DeckInfo";

export function useInfo() {
  return useQuery({ queryKey: ["info"], queryFn: () => api<DeckInfo>("/api/info") });
}

export function useAccount(): string | undefined {
  return useSearch().get("account") ?? undefined;
}

/** All, then one tab per account in deck.yaml. They filter every section. */
export function AccountTabs() {
  const info = useInfo();
  const account = useAccount();
  const tabs = [{ id: undefined, label: "All" }, ...(info.data?.accounts ?? [])];
  if (tabs.length <= 2) return null;
  return (
    <div role="tablist" aria-label="Accounts" className="flex gap-1 overflow-x-auto">
      {tabs.map((t) => {
        const selected = t.id === account;
        return (
          <button
            key={t.id ?? "all"}
            type="button"
            role="tab"
            aria-selected={selected}
            onClick={() => setParam("account", t.id)}
            className={cn(
              "rounded-md px-3 py-1.5 text-sm whitespace-nowrap text-neutral transition-colors " +
                "duration-(--dur-quick) hover:text-ink",
              selected && "bg-paper-3 text-ink",
            )}
          >
            {t.label}
          </button>
        );
      })}
    </div>
  );
}
