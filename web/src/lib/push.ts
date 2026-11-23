import { guessDeviceName } from "@/lib/webauthn";
import type { PushSubscribe } from "@/types/PushSubscribe";

/** Running as the installed app. iOS reports `navigator.standalone` false in some installed apps, so the display
 * mode is checked first. */
export function installed(): boolean {
  return (
    window.matchMedia("(display-mode: standalone)").matches ||
    (navigator as Navigator & { standalone?: boolean }).standalone === true
  );
}

/** Whether this browser can take pushes. iOS has PushManager only in an app added to the Home Screen. */
export function pushPossible(): boolean {
  return "PushManager" in window && "serviceWorker" in navigator && "Notification" in window;
}

export function appleMobile(): boolean {
  return (
    /iPhone|iPad|iPod/.test(navigator.userAgent) ||
    (navigator.platform === "MacIntel" && navigator.maxTouchPoints > 1)
  );
}

function bytes(base64url: string): Uint8Array<ArrayBuffer> {
  const b64 = base64url.replace(/-/g, "+").replace(/_/g, "/");
  const raw = atob(b64 + "=".repeat((4 - (b64.length % 4)) % 4));
  const out = new Uint8Array(new ArrayBuffer(raw.length));
  for (let i = 0; i < raw.length; i++) out[i] = raw.charCodeAt(i);
  return out;
}

function sameKey(sub: PushSubscription, publicKey: string): boolean {
  const key = sub.options.applicationServerKey;
  if (!key) return false;
  const want = bytes(publicKey);
  const have = new Uint8Array(key);
  return have.length === want.length && have.every((b, i) => b === want[i]);
}

/** This device's push subscription, if it has one. */
export async function currentSubscription(): Promise<PushSubscription | null> {
  if (!pushPossible()) return null;
  const reg = await navigator.serviceWorker.getRegistration();
  return reg ? reg.pushManager.getSubscription() : null;
}

/** Ask for permission (call it straight from a tap: iOS insists) and subscribe with the deck's key. */
export async function subscribe(publicKey: string): Promise<PushSubscribe> {
  const permission = await Notification.requestPermission();
  if (permission !== "granted") {
    throw new Error(
      permission === "denied"
        ? "Notifications are blocked for Quietdeck. Allow them in this device’s settings, then try again."
        : "Notifications weren’t allowed.",
    );
  }
  const reg = await navigator.serviceWorker.ready;
  let sub = await reg.pushManager.getSubscription();
  // A subscription made with another deck's key can't take this deck's pushes.
  if (sub && !sameKey(sub, publicKey)) {
    await sub.unsubscribe();
    sub = null;
  }
  try {
    sub ??= await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: bytes(publicKey) });
  } catch (e) {
    const why = e instanceof Error ? e.message : String(e);
    throw new Error(`This browser wouldn’t subscribe to pushes (${why}). A private window can’t take them.`);
  }
  const json = sub.toJSON();
  if (!json.endpoint || !json.keys?.p256dh || !json.keys.auth) throw new Error("The browser gave no push keys.");
  return { endpoint: json.endpoint, p256dh: json.keys.p256dh, auth: json.keys.auth, device: guessDeviceName() };
}
