import type { ApiError } from "@/types/ApiError";

/** An error the deck answered with: its HTTP status and its message, written for the person to read. */
export class DeckError extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message);
  }
}

type Method = "GET" | "POST" | "PUT" | "PATCH" | "DELETE";

/** Calls the deck's REST API with the sign-in cookie. A 401 sends the app back to the sign-in screen. */
export async function api<T>(path: string, init: { method?: Method; body?: unknown } = {}): Promise<T> {
  const res = await fetch(path, {
    method: init.method ?? "GET",
    credentials: "same-origin",
    headers: init.body === undefined ? undefined : { "content-type": "application/json" },
    body: init.body === undefined ? undefined : JSON.stringify(init.body),
  });
  if (!res.ok) {
    let message = `${res.status} ${res.statusText}`;
    try {
      message = ((await res.json()) as ApiError).error;
    } catch {
      // A body that isn't the deck's JSON error (a proxy's page): the status line says enough.
    }
    if (res.status === 401) onSignedOut?.();
    throw new DeckError(res.status, message);
  }
  const text = await res.text();
  return (text ? JSON.parse(text) : undefined) as T;
}

let onSignedOut: (() => void) | undefined;

export function setSignedOutHandler(handler: () => void) {
  onSignedOut = handler;
}

export function query(params: Record<string, string | undefined>): string {
  const q = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) if (v !== undefined && v !== "") q.set(k, v);
  const s = q.toString();
  return s ? `?${s}` : "";
}
