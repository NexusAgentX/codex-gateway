import type { ReactNode } from "react";
import { cn } from "../../lib/utils";

export function Badge({ tone, children }: { tone: "good" | "bad" | "neutral"; children: ReactNode }) {
  return (
    <span
      className={cn(
        "inline-flex min-h-6 items-center rounded-full px-2 py-0.5 text-xs font-bold",
        tone === "good" && "bg-emerald-50 text-emerald-800",
        tone === "bad" && "bg-red-50 text-red-800",
        tone === "neutral" && "bg-zinc-100 text-zinc-600"
      )}
    >
      {children}
    </span>
  );
}
