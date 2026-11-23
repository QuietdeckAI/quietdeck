import { Markdown as Render } from "markdown-to-jsx/react";
import type { ComponentProps } from "react";
import { cn } from "@/lib/utils";

function Table(props: ComponentProps<"table">) {
  return (
    <div className="table-wrap">
      <table {...props} />
    </div>
  );
}

function Link(props: ComponentProps<"a">) {
  return <a {...props} target="_blank" rel="noreferrer noopener" />;
}

const OPTIONS = {
  // Agents quote HTML and XML all the time: it shows as text, never as markup.
  disableParsingRawHTML: true,
  forceBlock: true,
  overrides: { table: Table, a: Link },
};

/** Markdown an agent or a summary wrote. */
export function Markdown({ text, className }: { text: string; className?: string }) {
  return (
    <Render className={cn("md", className)} options={OPTIONS}>
      {text}
    </Render>
  );
}
