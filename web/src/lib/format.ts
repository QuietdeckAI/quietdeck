const time = new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" });
const day = new Intl.DateTimeFormat(undefined, { weekday: "short", day: "numeric", month: "short" });
const clockTime = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
const longDay = new Intl.DateTimeFormat(undefined, { weekday: "long", day: "numeric", month: "long" });

function startOfDay(d: Date) {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

/** "14:02" today, "Yesterday 09:10", then "Fri 20 Nov 09:10". */
export function when(iso: string | null | undefined): string {
  if (!iso) return "";
  const d = new Date(iso);
  const days = Math.round((startOfDay(new Date()) - startOfDay(d)) / 86_400_000);
  if (days === 0) return time.format(d);
  if (days === 1) return `Yesterday ${time.format(d)}`;
  return `${day.format(d)} ${time.format(d)}`;
}

/** The time alone, as the person's locale writes it ("14:02", "2:02 PM"), for rows under a day heading. */
export function clock(iso: string): string {
  return clockTime.format(new Date(iso));
}

/** A day heading for History: "Today", "Yesterday", "Friday 20 November". */
export function dayHeading(iso: string): string {
  const d = new Date(iso);
  const days = Math.round((startOfDay(new Date()) - startOfDay(d)) / 86_400_000);
  if (days === 0) return "Today";
  if (days === 1) return "Yesterday";
  return longDay.format(d);
}

export function dayKey(iso: string): string {
  const d = new Date(iso);
  return `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;
}

/** "an hour ago"-style wording for the quiet check-in line. */
export function ago(iso: string): string {
  const mins = Math.round((Date.now() - new Date(iso).getTime()) / 60_000);
  if (mins < 90) return `${mins} minutes ago`;
  const hours = Math.round(mins / 60);
  if (hours < 36) return `${hours} hours ago`;
  return `on ${when(iso)}`;
}

/** "1 hour ago", "3 days ago": how long ago in words, for a Work item's "(…)". */
export function agoWords(iso: string, now = Date.now()): string {
  const mins = Math.floor(Math.max(0, now - new Date(iso).getTime()) / 60_000);
  const n = (v: number, unit: string) => `${v} ${unit}${v === 1 ? "" : "s"} ago`;
  if (mins < 1) return "just now";
  if (mins < 60) return n(mins, "minute");
  const hours = Math.floor(mins / 60);
  if (hours < 24) return n(hours, "hour");
  return n(Math.floor(hours / 24), "day");
}

/** "23 min ago": how long ago, in whole minutes up to an hour, then hours, then days. */
export function agoShort(iso: string, now = Date.now()): string {
  const secs = Math.max(0, Math.round((now - new Date(iso).getTime()) / 1000));
  if (secs < 60) return "just now";
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins} min ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours} hr ago`;
  const days = Math.floor(hours / 24);
  return days === 1 ? "yesterday" : `${days} days ago`;
}

const stamp = new Intl.DateTimeFormat(undefined, {
  weekday: "short",
  day: "numeric",
  month: "short",
  hour: "numeric",
  minute: "2-digit",
  hour12: true,
});

/**
 * The time zone's abbreviation ("NZDT", "AEST", "BST", "PST"). A browser in another locale often knows it only as an
 * offset ("GMT+13"), so English locales that name more zones are tried before settling for the offset.
 */
function zoneName(d: Date): string {
  const zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
  const name = (locale: string | undefined) =>
    new Intl.DateTimeFormat(locale, { timeZone: zone, timeZoneName: "short" })
      .formatToParts(d)
      .find((p) => p.type === "timeZoneName")?.value ?? "";
  let fallback = "";
  for (const locale of [undefined, "en-NZ", "en-AU", "en-GB", "en-US", "en-IN", "en-ZA"]) {
    const n = name(locale);
    if (n && !/^(GMT|UTC)[+-]/.test(n)) return n;
    fallback ||= n;
  }
  return fallback;
}

/** "Wed 25 Nov, 10:13am NZDT", in the person's own time zone. */
export function stampWithZone(iso: string): string {
  const d = new Date(iso);
  const parts = Object.fromEntries(stamp.formatToParts(d).map((p) => [p.type, p.value]));
  const period = (parts.dayPeriod ?? "").toLowerCase().replace(/\s|\./g, "");
  return `${parts.weekday} ${parts.day} ${parts.month}, ${parts.hour}:${parts.minute}${period} ${zoneName(d)}`;
}
