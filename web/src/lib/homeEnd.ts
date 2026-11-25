import { useEffect } from "react";

/**
 * Home and End in a text box move the caret to the start and end of the current line (Shift extends the selection),
 * as on Windows and Linux. macOS would scroll the page instead and leave the caret where it was (Nathan, 25 November:
 * "why do my home and end keys no longer go to the beginning and end of a text input"). Outside a text box they keep
 * scrolling the page.
 */
export function useHomeEndInTextBoxes() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.key !== "Home" && e.key !== "End") || e.metaKey || e.ctrlKey || e.altKey) return;
      const el = e.target;
      const box =
        el instanceof HTMLTextAreaElement ||
        (el instanceof HTMLInputElement && ["text", "search", "url", "email", "tel", "password", ""].includes(el.type));
      if (!box) return;
      const input = el as HTMLTextAreaElement | HTMLInputElement;
      const value = input.value;
      const caret = input.selectionDirection === "backward" ? (input.selectionStart ?? 0) : (input.selectionEnd ?? 0);
      const lineStart = value.lastIndexOf("\n", caret - 1) + 1;
      const next = value.indexOf("\n", caret);
      const lineEnd = next === -1 ? value.length : next;
      const to = e.key === "Home" ? lineStart : lineEnd;
      e.preventDefault();
      if (e.shiftKey) {
        // Extend from the end of the selection that doesn't move.
        const anchor = input.selectionDirection === "backward" ? (input.selectionEnd ?? 0) : (input.selectionStart ?? 0);
        input.setSelectionRange(Math.min(anchor, to), Math.max(anchor, to), to < anchor ? "backward" : "forward");
      } else {
        input.setSelectionRange(to, to);
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, []);
}
