import { Switch as SwitchPrimitive } from "radix-ui";
import type { ComponentProps } from "react";
import { cn } from "@/lib/utils";

export function Switch({ className, ...props }: ComponentProps<typeof SwitchPrimitive.Root>) {
  return (
    <SwitchPrimitive.Root
      className={cn(
        "relative inline-flex h-6 w-10 shrink-0 cursor-pointer items-center rounded-full border border-rule-strong " +
          "bg-paper-3 transition-colors duration-(--dur-quick) data-[state=checked]:border-accent " +
          "data-[state=checked]:bg-accent disabled:cursor-not-allowed disabled:opacity-45",
        className,
      )}
      {...props}
    >
      <SwitchPrimitive.Thumb
        className={
          "block size-4.5 translate-x-0.5 rounded-full bg-muted transition-transform duration-(--dur-quick) " +
          "ease-(--ease-out) data-[state=checked]:translate-x-[1.1rem] data-[state=checked]:bg-accent-ink"
        }
      />
    </SwitchPrimitive.Root>
  );
}
