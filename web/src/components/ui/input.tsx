import type { ComponentProps } from "react";
import { cn } from "@/lib/utils";

export const fieldClass =
  "w-full rounded-md border border-rule bg-paper px-3 text-ink placeholder:text-neutral " +
  "transition-colors duration-(--dur-quick) hover:border-rule-strong focus-visible:border-accent " +
  "focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-accent disabled:opacity-45 " +
  "aria-invalid:border-danger";

export function Input({ className, ...props }: ComponentProps<"input">) {
  return <input className={cn(fieldClass, "h-10", className)} {...props} />;
}

export function Textarea({ className, ...props }: ComponentProps<"textarea">) {
  return <textarea className={cn(fieldClass, "field-sizing-content min-h-10 resize-none py-2 leading-normal", className)} {...props} />;
}
