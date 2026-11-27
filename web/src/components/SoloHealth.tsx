import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import type { DeckInfo } from "@/types/DeckInfo";

/** `/api/solo/health`: what's wrong with the runner or its agent, in words. */
type SoloHealth = { ok: boolean; problems: string[] };

const POLL_MS = 60_000;

/**
 * Quietdeck solo only: what's wrong with the runner and its agent right now, polled every minute. Empty when all is
 * well, and on any other deck (which has no such endpoint, so the browser never asks it).
 */
export function useSoloProblems(): string[] {
  const info = useQuery({ queryKey: ["info"], queryFn: () => api<DeckInfo>("/api/info") });
  const solo = info.data?.solo === true;
  const health = useQuery({
    queryKey: ["solo-health"],
    enabled: solo,
    refetchInterval: POLL_MS,
    // 503 is the endpoint's answer when something is stale, with the problems in its body: not an error here.
    queryFn: async (): Promise<SoloHealth> => {
      const res = await fetch("/api/solo/health", { credentials: "same-origin" });
      if (res.status !== 200 && res.status !== 503) {
        return { ok: false, problems: [`Can't check the runner (HTTP ${res.status})`] };
      }
      return (await res.json()) as SoloHealth;
    },
  });
  if (!solo) return [];
  if (health.isError) return ["Can't check the runner: the deck didn't answer"];
  return health.data?.problems ?? [];
}

/** A toast that stays while the runner or its agent is failing, and goes once they recover. */
export function SoloHealthToast({ problems }: { problems: string[] }) {
  return (
    <div
      role="alert"
      className="toast-in pointer-events-auto rounded-lg border border-rule-strong bg-paper-3 py-2.5 pr-2.5 pl-3.5 text-sm shadow-lg shadow-black/30"
    >
      <ul className="space-y-1">
        {problems.map((p) => (
          <li key={p} className="text-danger">
            {p}
          </li>
        ))}
      </ul>
      <p className="mt-1 text-xs text-neutral">
        Run <code>quietdeck-solo status</code> on this machine for more.
      </p>
    </div>
  );
}

/** The same toast where there's no report yet (a new solo deck, the moment it matters most), in the same place. */
export function SoloHealthStack() {
  const problems = useSoloProblems();
  if (problems.length === 0) return null;
  return (
    <div className="pointer-events-none fixed top-[73px] right-4 z-30 flex w-[min(24rem,calc(100vw-2rem))] flex-col gap-2">
      <SoloHealthToast problems={problems} />
    </div>
  );
}
