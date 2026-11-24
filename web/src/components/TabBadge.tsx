import { useQuery } from "@tanstack/react-query";
import { useEffect } from "react";
import { api } from "@/lib/api";
import type { AuthState } from "@/types/AuthState";
import type { Inbox } from "@/types/Inbox";
import type { Settings } from "@/types/Settings";

const BASE_ICON = "/icon.svg";
const BASE_TITLE = "Quietdeck";
const SIZE = 64;

let base: Promise<HTMLImageElement> | undefined;
function baseIcon(): Promise<HTMLImageElement> {
  base ??= new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = reject;
    img.src = BASE_ICON;
  });
  return base;
}

/** Gmail's tab badge: white, with black figures. */
const BADGE_FILL = "#ffffff";
const BADGE_TEXT = "#000000";

/** `label` in black on a transparent canvas, and where its ink starts and ends, in fractional pixels: an edge row
 * counts for as much of a row as it's covered. Chrome snaps text to whole pixels, so the caller places this canvas
 * at the exact offset that centres the ink. */
function inkedLabel(label: string, font: string): { canvas: HTMLCanvasElement; top: number; bottom: number } | null {
  const canvas = document.createElement("canvas");
  canvas.width = SIZE;
  canvas.height = SIZE;
  const ctx = canvas.getContext("2d");
  if (!ctx) return null;
  ctx.font = font;
  ctx.fillStyle = BADGE_TEXT;
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillText(label, SIZE / 2, SIZE / 2);
  const data = ctx.getImageData(0, 0, SIZE, SIZE).data;
  const cover: number[] = [];
  for (let row = 0; row < SIZE; row++) {
    let most = 0;
    for (let col = 0; col < SIZE; col++) most = Math.max(most, data[(row * SIZE + col) * 4 + 3]);
    cover.push(most / 255);
  }
  const first = cover.findIndex((c) => c > 0);
  if (first < 0) return null;
  const last = cover.length - 1 - [...cover].reverse().findIndex((c) => c > 0);
  return { canvas, top: first + 1 - cover[first], bottom: last + cover[last] };
}

/** The base icon with the count in a white badge across its lower half, as Gmail draws its unread count: as tall as
 * Gmail's, with figures about two thirds of its height, centred on their ink, and as wide as the figures need (two or
 * three fill the icon). */
async function badgedIcon(label: string): Promise<string> {
  const img = await baseIcon();
  const canvas = document.createElement("canvas");
  canvas.width = SIZE;
  canvas.height = SIZE;
  const ctx = canvas.getContext("2d");
  if (!ctx) return BASE_ICON;
  ctx.drawImage(img, 0, 0, SIZE, SIZE);
  const h = 36;
  let size = 34;
  const font = () => `500 ${size}px "Geist Variable", ui-sans-serif, sans-serif`;
  ctx.font = font();
  while (size > 20 && ctx.measureText(label).width > SIZE - 4) {
    size -= 1;
    ctx.font = font();
  }
  const w = Math.min(SIZE, Math.max(h, ctx.measureText(label).width + 8));
  const x = SIZE - w;
  const y = SIZE - h;
  ctx.beginPath();
  ctx.roundRect(x, y, w, h, 9);
  ctx.fillStyle = BADGE_FILL;
  ctx.fill();
  // Centred on the figures' drawn ink, so the space above them matches the space below.
  const ink = inkedLabel(label, ctx.font);
  if (ink) ctx.drawImage(ink.canvas, x + w / 2 - SIZE / 2, y + h / 2 - (ink.top + ink.bottom) / 2);
  return canvas.toDataURL("image/png");
}

function setIcon(href: string) {
  let link = document.querySelector<HTMLLinkElement>('link[rel="icon"]');
  if (!link) {
    link = document.createElement("link");
    link.rel = "icon";
    document.head.appendChild(link);
  }
  link.type = href.startsWith("data:") ? "image/png" : "image/svg+xml";
  link.href = href;
}

/** The installed app's icon badge (the Badging API), where the browser has one. */
function appBadge(count: number) {
  if (!("setAppBadge" in navigator)) return;
  const done = count > 0 ? navigator.setAppBadge(count) : navigator.clearAppBadge();
  // Refused without notification permission on some browsers: the tab's own icon still shows the count.
  done.catch(() => undefined);
}

/** How many cards wait in the Inbox, on the tab's icon, the installed app's icon and in its title, while "Show the pending count on the tab
 * icon" is on (Settings). Nothing at zero. The Inbox's own live updates keep it current. */
export function TabBadge() {
  const auth = useQuery({ queryKey: ["auth"], queryFn: () => api<AuthState>("/api/auth/state") });
  const signedIn = auth.data?.signed_in === true;
  const settings = useQuery({
    queryKey: ["settings"],
    queryFn: () => api<Settings>("/api/settings"),
    enabled: signedIn,
  });
  const on = signedIn && settings.data?.show_pending_count === true;
  const inbox = useQuery({
    queryKey: ["inbox", null],
    queryFn: () => api<Inbox>("/api/inbox"),
    enabled: on,
  });
  const d = inbox.data;
  const count = on && d ? d.actions.length + d.suggestions.length + d.archive_list.length + d.sent.length : 0;

  useEffect(() => {
    appBadge(count);
    let current = true;
    if (count === 0) {
      setIcon(BASE_ICON);
      document.title = BASE_TITLE;
      return;
    }
    const label = count > 99 ? "99+" : String(count);
    document.title = `(${label}) ${BASE_TITLE}`;
    void document.fonts.ready
      .then(() => badgedIcon(label))
      .then((href) => current && setIcon(href))
      .catch(() => current && setIcon(BASE_ICON));
    return () => {
      current = false;
    };
  }, [count]);
  return null;
}
