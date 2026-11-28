import { useSyncExternalStore } from "react";

// Four screens don't need a router library: the path is the screen, and links push history.

const listeners = new Set<() => void>();

function subscribe(fn: () => void) {
  listeners.add(fn);
  window.addEventListener("popstate", fn);
  return () => {
    listeners.delete(fn);
    window.removeEventListener("popstate", fn);
  };
}

export function navigate(to: string, opts: { replace?: boolean } = {}) {
  if (to === window.location.pathname + window.location.search) return;
  if (opts.replace) window.history.replaceState(null, "", to);
  else window.history.pushState(null, "", to);
  for (const fn of listeners) fn();
  window.scrollTo({ top: 0 });
}

export function usePath(): string {
  return useSyncExternalStore(subscribe, () => window.location.pathname);
}

export function useSearch(): URLSearchParams {
  const search = useSyncExternalStore(subscribe, () => window.location.search);
  return new URLSearchParams(search);
}

/** Replaces a query parameter without a new history entry (the account tab). */
export function setParam(name: string, value: string | undefined) {
  const params = new URLSearchParams(window.location.search);
  if (value) params.set(name, value);
  else params.delete(name);
  const s = params.toString();
  window.history.replaceState(null, "", window.location.pathname + (s ? `?${s}` : ""));
  for (const fn of listeners) fn();
}
