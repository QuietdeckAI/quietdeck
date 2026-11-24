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
