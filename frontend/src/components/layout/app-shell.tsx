import { Activity, BarChart3, Boxes, Gauge, KeyRound, ListChecks, LogOut, Server, Settings, Users, type LucideIcon } from "lucide-react";
import { NavLink, Outlet } from "react-router-dom";
import { useSession } from "../../lib/auth/session";
import { cn } from "../../lib/utils";
import { Button } from "../ui/button";

type Page = {
  path: string;
  label: string;
  icon: LucideIcon;
  adminOnly?: boolean;
};

export const pages: Page[] = [
  { path: "/overview", label: "Overview", icon: Gauge },
  { path: "/usage", label: "Usage", icon: BarChart3 },
  { path: "/requests", label: "Requests", icon: ListChecks },
  { path: "/api-keys", label: "API Keys", icon: KeyRound },
  { path: "/upstreams", label: "Upstreams", icon: Server, adminOnly: true },
  { path: "/models", label: "Models", icon: Boxes, adminOnly: true },
  { path: "/users", label: "Users", icon: Users, adminOnly: true },
  { path: "/settings", label: "Settings", icon: Settings, adminOnly: true }
];

export function AppShell() {
  const { session, logout } = useSession();
  if (!session) return null;

  return (
    <div className="grid min-h-screen w-full max-w-[100vw] grid-cols-[240px_minmax(0,1fr)] overflow-x-hidden max-[760px]:grid-cols-1">
      <aside className="flex min-w-0 flex-col gap-5 overflow-hidden border-r border-zinc-200 bg-white p-4 max-[760px]:sticky max-[760px]:top-0 max-[760px]:z-10 max-[760px]:border-b max-[760px]:border-r-0 max-[760px]:p-3">
        <div className="flex min-h-9 min-w-0 items-center gap-2 px-2 font-bold text-zinc-950">
          <Activity size={20} />
          <span className="truncate">codex-gateway</span>
        </div>
        <nav className="flex min-w-0 flex-col gap-1 max-[760px]:flex-row max-[760px]:overflow-x-auto max-[760px]:overflow-y-hidden max-[760px]:pb-0.5">
          {pages.map((page) => {
            const Icon = page.icon;
            return (
              <NavLink
                key={page.path}
                to={page.path}
                title={page.adminOnly ? "Admin only" : undefined}
                className={({ isActive }) =>
                  cn(
                    "flex min-h-9 min-w-0 items-center gap-2 rounded-md px-2.5 py-2 text-sm text-zinc-600 no-underline max-[760px]:shrink-0",
                    isActive ? "bg-zinc-100 text-zinc-950" : "hover:bg-zinc-50 hover:text-zinc-950"
                  )
                }
              >
                <Icon size={17} />
                <span className="truncate">{page.label}</span>
              </NavLink>
            );
          })}
        </nav>
      </aside>
      <main className="min-w-0 max-w-full overflow-x-hidden px-6 py-5 max-[760px]:w-full max-[760px]:max-w-[100vw] max-[760px]:px-3 max-[760px]:py-4">
        <header className="mb-5 flex max-w-[1220px] min-w-0 items-center justify-between gap-4 max-[760px]:items-start">
          <div className="flex min-w-0 flex-wrap items-center gap-2">
            <strong className="min-w-0 break-all text-sm text-zinc-950">{session.user.email}</strong>
            <span className="rounded-full bg-zinc-100 px-2 py-1 text-xs font-medium text-zinc-600">{session.user.role}</span>
          </div>
          <Button type="button" onClick={logout}>
            <LogOut size={16} />
            Logout
          </Button>
        </header>
        <Outlet />
      </main>
    </div>
  );
}
