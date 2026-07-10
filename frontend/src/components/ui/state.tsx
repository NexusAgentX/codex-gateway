import { RefreshCw, ShieldAlert } from "lucide-react";
import type { ReactNode } from "react";
import { ApiClientError } from "../../lib/api/client";
import { messageForError } from "../../lib/format";
import { cn } from "../../lib/utils";

export function StateBlock({ children, tone = "default", className }: { children: ReactNode; tone?: "default" | "error" | "empty"; className?: string }) {
  return (
    <div
      className={cn(
        "flex min-h-16 min-w-0 items-center gap-2 overflow-wrap-anywhere rounded-lg border bg-white p-4 text-sm text-zinc-600",
        tone === "default" && "border-zinc-200",
        tone === "error" && "border-red-200 bg-red-50 text-red-800",
        tone === "empty" && "justify-center border-dashed border-zinc-300 text-zinc-500",
        className
      )}
    >
      {children}
    </div>
  );
}

export function LoadingState() {
  return (
    <StateBlock>
      <RefreshCw className="spin" size={18} />
      Loading
    </StateBlock>
  );
}

export function UnauthorizedState({ message }: { message: string }) {
  return (
    <StateBlock tone="error">
      <ShieldAlert size={18} />
      {message}
    </StateBlock>
  );
}

export function ErrorState({ error }: { error: unknown }) {
  if (error instanceof ApiClientError && (error.status === 401 || error.status === 403)) {
    return <UnauthorizedState message={error.message} />;
  }
  return <StateBlock tone="error">{messageForError(error)}</StateBlock>;
}

export function EmptyState({ text }: { text: string }) {
  return <StateBlock tone="empty">{text}</StateBlock>;
}
