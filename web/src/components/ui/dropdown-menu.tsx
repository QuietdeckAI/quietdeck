import { DropdownMenu as Menu } from "radix-ui";
import type { ComponentProps } from "react";
import { cn } from "@/lib/utils";

export const DropdownMenu = Menu.Root;
export const DropdownMenuTrigger = Menu.Trigger;

export function DropdownMenuContent({ className, sideOffset = 4, ...props }: ComponentProps<typeof Menu.Content>) {
  return (
    <Menu.Portal>
      <Menu.Content
        sideOffset={sideOffset}
        align="end"
        className={cn(
          "z-50 min-w-44 rounded-md border border-rule-strong bg-paper-3 p-1 text-sm text-ink shadow-xl " +
            "shadow-black/40",
          className,
        )}
        {...props}
      />
    </Menu.Portal>
  );
}

export function DropdownMenuItem({ className, ...props }: ComponentProps<typeof Menu.Item>) {
  return (
    <Menu.Item
      className={cn(
        "flex cursor-default select-none items-center gap-2 rounded-sm px-2 py-1.5 outline-none " +
          "data-[disabled]:opacity-45 data-[highlighted]:bg-paper-2 [&_svg]:size-4 [&_svg]:text-neutral",
        className,
      )}
      {...props}
    />
  );
}
