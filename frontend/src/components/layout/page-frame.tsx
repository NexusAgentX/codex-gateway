import { RefreshCw, type LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { Button } from "../ui/button";

export function PageFrame({
  title,
  icon: Icon,
  onRefresh,
  refreshing,
  children
}: {
  title: string;
  icon: LucideIcon;
  onRefresh?: () => void;
  refreshing?: boolean;
  children: ReactNode;
}) {
  return (
    <section className="grid w-full max-w-[1220px] gap-4">
      <header className="flex min-w-0 items-center justify-between gap-4">
        <h1 className="min-w-0 text-2xl font-semibold tracking-normal text-zinc-950">{title}</h1>
        {onRefresh ? (
          <Button type="button" size="icon" onClick={onRefresh} aria-label={`Refresh ${title}`} disabled={refreshing}>
            <RefreshCw className={refreshing ? "spin" : ""} size={18} />
          </Button>
        ) : (
          <Icon className="shrink-0 text-zinc-500" size={22} />
        )}
      </header>
      {children}
    </section>
  );
}
