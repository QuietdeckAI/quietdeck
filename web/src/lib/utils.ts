import { type ClassValue, clsx } from "clsx";
import { twMerge } from "tailwind-merge";
import type { KeyboardEvent } from "react";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

/**
 * A text box's Enter key: Enter sends, Shift+Enter makes a new line. Enter while an input method is composing (an
 * accent, Japanese) belongs to the input method.
 */
export function enterSends(send: () => void) {
  return (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key !== "Enter" || e.shiftKey || e.nativeEvent.isComposing) return;
    e.preventDefault();
    send();
  };
}
