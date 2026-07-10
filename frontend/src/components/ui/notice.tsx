import { cn } from "../../lib/utils";

export function Notice({ tone = "note", children }: { tone?: "note" | "error"; children: React.ReactNode }) {
  return (
    <div
      className={cn(
        "rounded-md border px-3 py-2 text-sm",
        tone === "note" && "border-emerald-100 bg-emerald-50 text-emerald-900",
        tone === "error" && "border-red-200 bg-red-50 text-red-800"
      )}
    >
      {children}
    </div>
  );
}
