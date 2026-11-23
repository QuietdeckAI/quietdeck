import { cva, type VariantProps } from "class-variance-authority";
import { Loader2 } from "lucide-react";
import { Slot } from "radix-ui";
import type { ComponentProps } from "react";
import { cn } from "@/lib/utils";

const buttonVariants = cva(
  "inline-flex shrink-0 items-center justify-center gap-2 whitespace-nowrap rounded-md font-medium " +
    "transition-[background-color,border-color,color,transform] duration-(--dur-quick) ease-(--ease-out) " +
    "active:translate-y-px disabled:pointer-events-none disabled:opacity-45 aria-busy:cursor-progress " +
    "[&_svg]:size-4 [&_svg]:shrink-0",
  {
    variants: {
      variant: {
        primary: "bg-accent text-accent-ink hover:bg-focus",
        secondary: "border border-rule-strong bg-paper-2 text-ink hover:border-neutral hover:bg-paper-3",
        quiet: "text-muted hover:bg-paper-3 hover:text-ink",
        danger: "border border-danger/50 text-danger hover:bg-danger-wash",
      },
      size: {
        sm: "h-8 px-3 text-sm",
        md: "h-10 px-4 text-base",
        icon: "size-8",
      },
    },
    defaultVariants: { variant: "secondary", size: "md" },
  },
);

type ButtonProps = ComponentProps<"button"> &
  VariantProps<typeof buttonVariants> & { asChild?: boolean; loading?: boolean };

export function Button({ className, variant, size, asChild, loading, children, disabled, ...props }: ButtonProps) {
  const Comp = asChild ? Slot.Root : "button";
  return (
    <Comp
      className={cn(buttonVariants({ variant, size }), className)}
      disabled={disabled || loading}
      aria-busy={loading || undefined}
      {...props}
    >
      {asChild ? (
        // Slot takes exactly one child: the element it renders as.
        children
      ) : (
        <>
          {loading && <Loader2 className="animate-spin motion-reduce:animate-none" aria-hidden />}
          {children}
        </>
      )}
    </Comp>
  );
}
