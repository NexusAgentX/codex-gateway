import {
  Activity,
  Boxes,
  Gauge,
  KeyRound,
  ListChecks,
  Settings,
  Server,
  Users
} from "lucide-react";
import type { ComponentType } from "react";
import { NavLink, Navigate, Route, Routes } from "react-router-dom";

type Page = {
  path: string;
  label: string;
  icon: ComponentType<{ size?: number }>;
  rows: string[];
};

const pages: Page[] = [
  {
    path: "/overview",
    label: "Overview",
    icon: Gauge,
    rows: ["Today requests", "Token usage", "Error rate", "Healthy upstreams"]
  },
  {
    path: "/requests",
    label: "Requests",
    icon: ListChecks,
    rows: ["Request id", "Model", "Upstream", "Status", "Latency", "Usage source"]
  },
  {
    path: "/api-keys",
    label: "API Keys",
    icon: KeyRound,
    rows: ["Name", "Prefix", "Status", "Last used", "Expires"]
  },
  {
    path: "/upstreams",
    label: "Upstreams",
    icon: Server,
    rows: ["Name", "Base URL", "Priority", "Weight", "Health", "Timeout"]
  },
  {
    path: "/models",
    label: "Models",
    icon: Boxes,
    rows: ["Public name", "Visible", "Enabled", "Mappings", "Fallback"]
  },
  {
    path: "/users",
    label: "Users",
    icon: Users,
    rows: ["Email", "Role", "Status", "Keys", "Last login"]
  },
  {
    path: "/settings",
    label: "Settings",
    icon: Settings,
    rows: ["Route strategy", "Retention", "CORS", "Debug headers"]
  }
];

export function App() {
  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <Activity size={20} />
          <span>codex-gateway</span>
        </div>
        <nav>
          {pages.map((page) => {
            const Icon = page.icon;
            return (
              <NavLink key={page.path} to={page.path}>
                <Icon size={17} />
                <span>{page.label}</span>
              </NavLink>
            );
          })}
        </nav>
      </aside>
      <main>
        <Routes>
          <Route path="/" element={<Navigate to="/overview" replace />} />
          {pages.map((page) => (
            <Route key={page.path} path={page.path} element={<Panel page={page} />} />
          ))}
        </Routes>
      </main>
    </div>
  );
}

function Panel({ page }: { page: Page }) {
  const Icon = page.icon;
  return (
    <section className="panel">
      <header className="panel-header">
        <div>
          <h1>{page.label}</h1>
          <p>Phase-one admin surface wired for backend JSON APIs.</p>
        </div>
        <button type="button" aria-label={`Refresh ${page.label}`}>
          <Icon size={18} />
        </button>
      </header>
      <div className="table-shell">
        <table>
          <thead>
            <tr>
              {page.rows.map((row) => (
                <th key={row}>{row}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            <tr>
              {page.rows.map((row) => (
                <td key={row}>-</td>
              ))}
            </tr>
          </tbody>
        </table>
      </div>
    </section>
  );
}
