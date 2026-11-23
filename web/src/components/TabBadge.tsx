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

function token(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

/** The base icon with the count in a pill at the bottom right: the accent with dark figures, ringed in the icon's own
 * dark so it stays clear on light and dark tab strips at 16 px. */
async function badgedIcon(label: string): Promise<string> {
  const img = await baseIcon();
  const canvas = document.createElement("canvas");
  canvas.width = SIZE;
  canvas.height = SIZE;
  const ctx = canvas.getContext("2d");
  if (!ctx) return BASE_ICON;
  ctx.drawImage(img, 0, 0, SIZE, SIZE);
  const h = 50;
  ctx.font = `700 ${label.length > 2 ? 30 : label.length > 1 ? 40 : 44}px "Geist Variable", ui-sans-serif, sans-serif`;
  const w = Math.min(SIZE, Math.max(h, ctx.measureText(label).width + 10));
  const x = SIZE - w;
  const y = SIZE - h;
  ctx.beginPath();
  ctx.roundRect(x, y, w, h, h / 2);
  ctx.fillStyle = token("--color-accent");
  ctx.fill();
  ctx.lineWidth = 4;
  ctx.strokeStyle = token("--color-paper");
  ctx.stroke();
  ctx.fillStyle = token("--color-accent-ink");
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillText(label, x + w / 2, y + h / 2 + 3);
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
