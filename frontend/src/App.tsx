import {
  Activity,
  Boxes,
  Check,
  Copy,
  Gauge,
  KeyRound,
  ListChecks,
  LogOut,
  Plus,
  RefreshCw,
  Save,
  Server,
  Settings,
  ShieldAlert,
  Trash2,
  Users,
  X
} from "lucide-react";
import type { ComponentType, DependencyList, FormEvent, ReactNode } from "react";
import { useEffect, useState } from "react";
import { NavLink, Navigate, Route, Routes } from "react-router-dom";
import {
  ApiClientError,
  apiFetch,
  type ApiKeySummary,
  type DailyUsage,
  type GatewayMetrics,
  type AdminLimitState,
  type LimitPolicy,
  type LimitSubjectState,
  type LoginResponse,
  type LoginUser,
  type Model,
  type ModelMapping,
  type OverviewResponse,
  type RequestLog,
  type SettingsSummary,
  type Upstream,
  type UserLimitState,
  type User
} from "./api";

type Session = {
  token: string;
  user: LoginUser;
};

type Page = {
  path: string;
  label: string;
  icon: ComponentType<{ size?: number }>;
  adminOnly?: boolean;
};

type Resource<T> =
  | { status: "loading"; data?: T; error?: undefined }
  | { status: "ready"; data: T; error?: undefined }
  | { status: "error"; data?: undefined; error: Error };

type LimitMode = "inherit" | "limited" | "unlimited";
type LimitPatchPayload = {
  mode: LimitMode;
  value?: number;
};

const sessionKey = "codex-gateway-session";

const pages: Page[] = [
  { path: "/overview", label: "Overview", icon: Gauge },
  { path: "/requests", label: "Requests", icon: ListChecks },
  { path: "/api-keys", label: "API Keys", icon: KeyRound },
  { path: "/upstreams", label: "Upstreams", icon: Server, adminOnly: true },
  { path: "/models", label: "Models", icon: Boxes, adminOnly: true },
  { path: "/users", label: "Users", icon: Users, adminOnly: true },
  { path: "/settings", label: "Settings", icon: Settings, adminOnly: true }
];

export function App() {
  const [session, setSession] = useState<Session | null>(() => readStoredSession());
  const [checkingSession, setCheckingSession] = useState(Boolean(session));

  useEffect(() => {
    if (!session) {
      setCheckingSession(false);
      return;
    }
    let active = true;
    apiFetch<{ email: string; role: string; user_id: string }>("/api/me", { token: session.token })
      .then((me) => {
        if (!active) return;
        const next = {
          ...session,
          user: { id: me.user_id, email: me.email, role: me.role }
        };
        setSession(next);
        storeSession(next);
      })
      .catch((error) => {
        if (!active) return;
        if (error instanceof ApiClientError && error.status === 401) {
          clearStoredSession();
          setSession(null);
        }
      })
      .finally(() => active && setCheckingSession(false));
    return () => {
      active = false;
    };
  }, []);

  async function logout() {
    clearStoredSession();
    setSession(null);
  }

  function onLogin(login: LoginResponse) {
    const next = { token: login.token, user: login.user };
    storeSession(next);
    setSession(next);
  }

  if (!session) {
    return <LoginPage onLogin={onLogin} />;
  }

  if (checkingSession) {
    return (
      <div className="screen-center">
        <Activity className="spin" size={22} />
        <span>Checking session</span>
      </div>
    );
  }

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
              <NavLink key={page.path} to={page.path} title={page.adminOnly ? "Admin only" : undefined}>
                <Icon size={17} />
                <span>{page.label}</span>
              </NavLink>
            );
          })}
        </nav>
      </aside>
      <main>
        <header className="topbar">
          <div>
            <strong>{session.user.email}</strong>
            <span>{session.user.role}</span>
          </div>
          <button type="button" className="icon-text" onClick={logout}>
            <LogOut size={16} />
            Logout
          </button>
        </header>
        <Routes>
          <Route path="/" element={<Navigate to="/overview" replace />} />
          <Route path="/overview" element={<OverviewPage session={session} />} />
          <Route path="/requests" element={<RequestsPage session={session} />} />
          <Route path="/api-keys" element={<ApiKeysPage session={session} />} />
          <Route path="/upstreams" element={<AdminGate session={session}><UpstreamsPage session={session} /></AdminGate>} />
          <Route path="/models" element={<AdminGate session={session}><ModelsPage session={session} /></AdminGate>} />
          <Route path="/users" element={<AdminGate session={session}><UsersPage session={session} /></AdminGate>} />
          <Route path="/settings" element={<AdminGate session={session}><SettingsPage session={session} /></AdminGate>} />
        </Routes>
      </main>
    </div>
  );
}

function LoginPage({ onLogin }: { onLogin: (login: LoginResponse) => void }) {
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  async function submit(event: FormEvent) {
    event.preventDefault();
    setLoading(true);
    setError(null);
    try {
      const login = await apiFetch<LoginResponse>("/api/login", {
        method: "POST",
        body: { email, password }
      });
      onLogin(login);
    } catch (err) {
      setError(messageForError(err));
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="login-screen">
      <form className="login-panel" onSubmit={submit}>
        <div className="brand login-brand">
          <Activity size={22} />
          <span>codex-gateway</span>
        </div>
        <label>
          Email
          <input name="email" value={email} onChange={(event) => setEmail(event.target.value)} autoComplete="username" required />
        </label>
        <label>
          Password
          <input
            value={password}
            name="password"
            onChange={(event) => setPassword(event.target.value)}
            type="password"
            autoComplete="current-password"
            required
          />
        </label>
        {error ? <div className="inline-error">{error}</div> : null}
        <button type="submit" className="primary" disabled={loading}>
          {loading ? <RefreshCw className="spin" size={16} /> : <KeyRound size={16} />}
          Sign in
        </button>
      </form>
    </div>
  );
}

function OverviewPage({ session }: { session: Session }) {
  const [tick, setTick] = useState(0);
  const resource = useResource(async () => {
    if (isAdmin(session)) {
      const [dailyUsage, recentRequests, metrics, limits] = await Promise.all([
        apiFetch<DailyUsage[]>("/api/admin/usage/daily", { token: session.token }),
        apiFetch<RequestLog[]>("/api/admin/requests", { token: session.token }),
        apiFetch<GatewayMetrics>("/api/admin/metrics", { token: session.token }),
        apiFetch<AdminLimitState>("/api/admin/limits", { token: session.token })
      ]);
      return { user: null, daily_usage: dailyUsage, recent_requests: recentRequests, metrics, limits };
    }
    const [overview, limits] = await Promise.all([
      apiFetch<OverviewResponse>("/api/overview", { token: session.token }),
      apiFetch<UserLimitState>("/api/limits", { token: session.token })
    ]);
    return { ...overview, metrics: null, limits };
  }, [session.token, session.user.role, tick]);

  return (
    <PageFrame title="Overview" icon={Gauge} onRefresh={() => setTick((value) => value + 1)}>
      <ResourceState resource={resource}>
        {(overview) => {
          const totals = summarizeUsage(overview.daily_usage);
          const errors = overview.recent_requests.filter((request) => (request.status_code ?? 500) >= 400).length;
          const failingUpstreams = overview.metrics?.upstream_health.filter((upstream) => upstream.last_health_status === "down" || upstream.last_health_status === "degraded") ?? [];
          return (
            <>
              <div className="stat-grid">
                <Stat label="Requests" value={formatNumber(overview.metrics?.request_count ?? totals.requests)} />
                <Stat label="Tokens" value={formatNumber(overview.metrics?.token_usage.total_tokens ?? totals.tokens)} />
                <Stat label="Errors" value={formatNumber(overview.metrics?.error_count ?? errors)} />
                <Stat label="Avg latency" value={overview.metrics?.latency.avg_ms ? `${Math.round(overview.metrics.latency.avg_ms)} ms` : totals.requests ? `${Math.round(totals.latency / totals.requests)} ms` : "-"} />
              </div>
              <LimitSummary state={"user" in overview.limits ? overview.limits.user : overview.limits.users[0]} />
              {overview.metrics ? (
                <Table
                  empty="No unhealthy upstreams."
                  columns={["Upstream", "Health", "Errors", "Last down", "Recent issue"]}
                  rows={failingUpstreams.map((upstream) => [
                    upstream.name,
                    <Badge key="health" tone="bad">{upstream.last_health_status}</Badge>,
                    formatNumber(upstream.error_count),
                    formatDate(upstream.last_down_at ?? upstream.last_degraded_at),
                    latestErrorSample(upstream.recent_error_samples)
                  ])}
                />
              ) : null}
              <Table
                empty="No recent requests yet."
                columns={["Started", "Request ID", "Status", "Model", "Upstream", "Latency", "Usage"]}
                rows={overview.recent_requests.slice(0, 12).map((request) => [
                  formatDate(request.started_at),
                  request.request_id,
                  <Badge key="status" tone={statusTone(request.status_code)}>{request.status_code ?? "pending"}</Badge>,
                  request.model_id ?? "-",
                  request.upstream_id ?? "-",
                  `${request.latency_ms} ms`,
                  `${formatNumber(request.total_tokens)} tokens`
                ])}
              />
            </>
          );
        }}
      </ResourceState>
    </PageFrame>
  );
}

function ApiKeysPage({ session }: { session: Session }) {
  const [tick, setTick] = useState(0);
  const [name, setName] = useState("");
  const [expiresAt, setExpiresAt] = useState("");
  const [userId, setUserId] = useState(session.user.id);
  const [plaintext, setPlaintext] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const admin = isAdmin(session);

  const resource = useResource(async () => {
    const [keys, users, limits] = await Promise.all([
      apiFetch<ApiKeySummary[]>(admin ? "/api/admin/api-keys" : "/api/api-keys", { token: session.token }),
      admin ? apiFetch<User[]>("/api/admin/users", { token: session.token }) : Promise.resolve([]),
      admin ? apiFetch<AdminLimitState>("/api/admin/limits", { token: session.token }) : apiFetch<UserLimitState>("/api/limits", { token: session.token })
    ]);
    return { keys, users, limits };
  }, [session.token, admin, tick]);

  async function createKey(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setActionError(null);
    setPlaintext(null);
    try {
      const expires_at = expiresAt ? new Date(expiresAt).toISOString() : null;
      const body = admin ? { user_id: userId, name, expires_at } : { name, expires_at };
      const created = await apiFetch<{ key: ApiKeySummary; plaintext: string }>(admin ? "/api/admin/api-keys" : "/api/api-keys", {
        method: "POST",
        token: session.token,
        body
      });
      setPlaintext(created.plaintext);
      setName("");
      setExpiresAt("");
      setTick((value) => value + 1);
    } catch (err) {
      setActionError(messageForError(err));
    } finally {
      setBusy(false);
    }
  }

  async function setStatus(id: string, status: "disable" | "revoke") {
    setBusy(true);
    setActionError(null);
    try {
      await apiFetch(`${admin ? "/api/admin" : "/api"}/api-keys/${id}/${status}`, {
        method: "POST",
        token: session.token
      });
      setTick((value) => value + 1);
    } catch (err) {
      setActionError(messageForError(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <PageFrame title="API Keys" icon={KeyRound} onRefresh={() => setTick((value) => value + 1)}>
      <form className="toolbar-form" onSubmit={createKey}>
        {admin ? (
          <select name="user_id" value={userId} onChange={(event) => setUserId(event.target.value)} aria-label="User">
            {resource.status === "ready" && resource.data.users.map((user) => (
              <option key={user.id} value={user.id}>{user.email}</option>
            ))}
          </select>
        ) : null}
        <input name="key_name" value={name} onChange={(event) => setName(event.target.value)} placeholder="Key name" required />
        <input name="expires_at" value={expiresAt} onChange={(event) => setExpiresAt(event.target.value)} type="datetime-local" aria-label="Expires at" />
        <button type="submit" className="primary" disabled={busy}>
          <Plus size={16} />
          Create
        </button>
      </form>
      {plaintext ? <OneTimeSecret value={plaintext} /> : null}
      {actionError ? <div className="inline-error">{actionError}</div> : null}
      <ResourceState resource={resource}>
        {({ keys, users, limits }) => (
          <>
            <Table
              empty="No API keys have been created."
              columns={["Name", "Owner", "Prefix", "Status", "Requests", "Tokens", "Rate", "Actions"]}
              rows={keys.map((key) => {
                const keyLimits = limits.api_keys.find((limit) => limit.subject_id === key.id);
                return [
                  key.name,
                  users.find((user) => user.id === key.user_id)?.email ?? key.user_id,
                  key.key_prefix,
                  <Badge key="status" tone={key.status === "active" ? "good" : "bad"}>{key.status}</Badge>,
                  keyLimits ? limitCell(keyLimits.request_quota) : "-",
                  keyLimits ? limitCell(keyLimits.token_budget) : "-",
                  keyLimits ? limitCell(keyLimits.rate_limit) : "-",
                  <div key="actions" className="row-actions">
                    <button type="button" onClick={() => setStatus(key.id, "disable")} disabled={busy || key.status !== "active"} title="Disable">
                      <X size={15} />
                    </button>
                    <button type="button" onClick={() => setStatus(key.id, "revoke")} disabled={busy || key.status === "revoked"} title="Revoke">
                      <Trash2 size={15} />
                    </button>
                  </div>
                ];
              })}
            />
            {admin ? (
              <div className="limit-editor-list">
                {keys.map((key) => {
                  const keyLimits = (limits as AdminLimitState).api_keys.find((limit) => limit.subject_id === key.id);
                  return keyLimits ? (
                    <LimitPolicyEditor
                      key={key.id}
                      title={`Key limits: ${key.name}`}
                      policy={keyLimits.policy}
                      compact
                      onSave={async (body) => {
                        await apiFetch(`/api/admin/api-keys/${key.id}/limits`, {
                          method: "PATCH",
                          token: session.token,
                          body
                        });
                        setTick((value) => value + 1);
                      }}
                    />
                  ) : null;
                })}
              </div>
            ) : null}
          </>
        )}
      </ResourceState>
    </PageFrame>
  );
}

function RequestsPage({ session }: { session: Session }) {
  const [tick, setTick] = useState(0);
  const [filters, setFilters] = useState({ user_id: "", key_id: "", model_id: "", upstream_id: "", status: "", from: "", to: "" });
  const admin = isAdmin(session);
  const resource = useResource(async () => {
    const requestPath = `${admin ? "/api/admin/requests" : "/api/requests"}${requestFilterQuery(filters)}`;
    const [requests, upstreams, models] = await Promise.all([
      apiFetch<RequestLog[]>(requestPath, { token: session.token }),
      admin ? apiFetch<Upstream[]>("/api/admin/upstreams", { token: session.token }) : Promise.resolve([]),
      admin ? apiFetch<Model[]>("/api/admin/models", { token: session.token }) : Promise.resolve([])
    ]);
    return { requests, upstreams, models };
  }, [session.token, admin, tick, filters.user_id, filters.key_id, filters.model_id, filters.upstream_id, filters.status, filters.from, filters.to]);

  return (
    <PageFrame title="Requests" icon={ListChecks} onRefresh={() => setTick((value) => value + 1)}>
      <div className="filter-grid">
        {admin ? <input name="filter_user_id" value={filters.user_id} onChange={(event) => setFilters({ ...filters, user_id: event.target.value })} placeholder="User ID" /> : null}
        <input name="filter_key_id" value={filters.key_id} onChange={(event) => setFilters({ ...filters, key_id: event.target.value })} placeholder="Key ID" />
        <input name="filter_model_id" value={filters.model_id} onChange={(event) => setFilters({ ...filters, model_id: event.target.value })} placeholder="Model ID" />
        <input name="filter_upstream_id" value={filters.upstream_id} onChange={(event) => setFilters({ ...filters, upstream_id: event.target.value })} placeholder="Upstream ID" />
        <input name="filter_status" value={filters.status} onChange={(event) => setFilters({ ...filters, status: event.target.value })} placeholder="Status" inputMode="numeric" />
        <input name="filter_from" value={filters.from} onChange={(event) => setFilters({ ...filters, from: event.target.value })} type="date" aria-label="From" />
        <input name="filter_to" value={filters.to} onChange={(event) => setFilters({ ...filters, to: event.target.value })} type="date" aria-label="To" />
        <button type="button" onClick={() => setFilters({ user_id: "", key_id: "", model_id: "", upstream_id: "", status: "", from: "", to: "" })}>
          <X size={16} />
          Clear
        </button>
      </div>
      <ResourceState resource={resource}>
        {({ requests, upstreams, models }) => {
          const upstreamNames = new Map(upstreams.map((upstream) => [upstream.id, upstream.name]));
          const modelNames = new Map(models.map((model) => [model.id, model.public_name]));
          return (
            <Table
              empty="No requests have been logged."
              columns={["Started", "Request ID", "Status", "Model", "Upstream", "Latency", "Usage", "Error code"]}
              rows={requests.map((request) => [
                formatDate(request.started_at),
                request.request_id,
                <Badge key="status" tone={statusTone(request.status_code)}>{request.status_code ?? "pending"}</Badge>,
                request.model_id ? modelNames.get(request.model_id) ?? request.model_id : "-",
                request.upstream_id ? upstreamNames.get(request.upstream_id) ?? request.upstream_id : "-",
                `${request.latency_ms} ms`,
                `${formatNumber(request.total_tokens)} (${request.usage_source})`,
                request.error_code ?? "-"
              ])}
            />
          );
        }}
      </ResourceState>
    </PageFrame>
  );
}

function UpstreamsPage({ session }: { session: Session }) {
  const [tick, setTick] = useState(0);
  const [draft, setDraft] = useState(upstreamDefaults());
  const [editing, setEditing] = useState<Upstream | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const resource = useResource(() => apiFetch<Upstream[]>("/api/admin/upstreams", { token: session.token }), [session.token, tick]);

  function edit(upstream: Upstream) {
    setEditing(upstream);
    setDraft({
      name: upstream.name,
      base_url: upstream.base_url,
      api_key: "",
      enabled: Boolean(upstream.enabled),
      priority: String(upstream.priority),
      weight: String(upstream.weight),
      timeout_ms: String(upstream.timeout_ms),
      max_retries: String(upstream.max_retries),
      health_check_path: upstream.health_check_path
    });
  }

  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setMessage(null);
    try {
      const body = upstreamBody(draft, Boolean(editing));
      await apiFetch(editing ? `/api/admin/upstreams/${editing.id}` : "/api/admin/upstreams", {
        method: editing ? "PATCH" : "POST",
        token: session.token,
        body
      });
      setDraft(upstreamDefaults());
      setEditing(null);
      setTick((value) => value + 1);
    } catch (err) {
      setMessage(messageForError(err));
    } finally {
      setBusy(false);
    }
  }

  async function checkHealth(id: string) {
    setBusy(true);
    setMessage(null);
    try {
      await apiFetch(`/api/admin/upstreams/${id}/health`, { method: "POST", token: session.token });
      setMessage("Health check completed.");
      setTick((value) => value + 1);
    } catch (err) {
      setMessage(messageForError(err));
      setTick((value) => value + 1);
    } finally {
      setBusy(false);
    }
  }

  async function disable(id: string) {
    setBusy(true);
    setMessage(null);
    try {
      await apiFetch(`/api/admin/upstreams/${id}/disable`, { method: "POST", token: session.token });
      setTick((value) => value + 1);
    } catch (err) {
      setMessage(messageForError(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <PageFrame title="Upstreams" icon={Server} onRefresh={() => setTick((value) => value + 1)}>
      <form className="edit-grid" onSubmit={submit}>
        <input name="upstream_name" value={draft.name} onChange={(event) => setDraft({ ...draft, name: event.target.value })} placeholder="Name" required />
        <input name="upstream_base_url" value={draft.base_url} onChange={(event) => setDraft({ ...draft, base_url: event.target.value })} placeholder="Base URL" required />
        <input name="upstream_api_key" value={draft.api_key} onChange={(event) => setDraft({ ...draft, api_key: event.target.value })} placeholder={editing ? "New API key optional" : "API key"} required={!editing} />
        <input name="upstream_health_check_path" value={draft.health_check_path} onChange={(event) => setDraft({ ...draft, health_check_path: event.target.value })} placeholder="/v1/models" />
        <NumberInput label="Priority" value={draft.priority} onChange={(value) => setDraft({ ...draft, priority: value })} />
        <NumberInput label="Weight" value={draft.weight} onChange={(value) => setDraft({ ...draft, weight: value })} />
        <NumberInput label="Timeout ms" value={draft.timeout_ms} onChange={(value) => setDraft({ ...draft, timeout_ms: value })} />
        <NumberInput label="Retries" value={draft.max_retries} onChange={(value) => setDraft({ ...draft, max_retries: value })} />
        <label className="checkline">
          <input name="upstream_enabled" type="checkbox" checked={draft.enabled} onChange={(event) => setDraft({ ...draft, enabled: event.target.checked })} />
          Enabled
        </label>
        <button type="submit" className="primary" disabled={busy}>
          <Save size={16} />
          {editing ? "Save" : "Create"}
        </button>
        {editing ? <button type="button" onClick={() => { setEditing(null); setDraft(upstreamDefaults()); }}>Cancel</button> : null}
      </form>
      {message ? <div className="inline-note">{message}</div> : null}
      <ResourceState resource={resource}>
        {(upstreams) => (
          <Table
            empty="No upstreams configured."
            columns={["Name", "Base URL", "Priority", "Health", "Last checked", "Recent issue", "Actions"]}
            rows={upstreams.map((upstream) => [
              upstream.name,
              upstream.base_url,
              upstream.priority,
              <Badge key="health" tone={upstream.last_health_status === "healthy" ? "good" : upstream.last_health_status === "unknown" ? "neutral" : "bad"}>{upstream.last_health_status}</Badge>,
              formatDate(upstream.last_health_checked_at),
              latestErrorSample(upstream.recent_error_samples),
              <div key="actions" className="row-actions">
                <button type="button" onClick={() => edit(upstream)} title="Edit"><Save size={15} /></button>
                <button type="button" onClick={() => checkHealth(upstream.id)} disabled={busy} title="Health check"><Activity size={15} /></button>
                <button type="button" onClick={() => disable(upstream.id)} disabled={busy || !upstream.enabled} title="Disable"><X size={15} /></button>
              </div>
            ])}
          />
        )}
      </ResourceState>
    </PageFrame>
  );
}

function ModelsPage({ session }: { session: Session }) {
  const [tick, setTick] = useState(0);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [createName, setCreateName] = useState("");
  const [message, setMessage] = useState<string | null>(null);
  const resource = useResource(async () => {
    const [models, upstreams] = await Promise.all([
      apiFetch<Model[]>("/api/admin/models", { token: session.token }),
      apiFetch<Upstream[]>("/api/admin/upstreams", { token: session.token })
    ]);
    const mappingPairs = await Promise.all(
      models.map(async (model) => [model.id, await apiFetch<ModelMapping[]>(`/api/admin/models/${model.id}/mappings`, { token: session.token })] as const)
    );
    return { models, upstreams, mappings: Object.fromEntries(mappingPairs) as Record<string, ModelMapping[]> };
  }, [session.token, tick]);

  async function createModel(event: FormEvent) {
    event.preventDefault();
    setMessage(null);
    try {
      await apiFetch<Model>("/api/admin/models", {
        method: "POST",
        token: session.token,
        body: { public_name: createName, description: null, enabled: true, visible_to_users: true }
      });
      setCreateName("");
      setTick((value) => value + 1);
    } catch (err) {
      setMessage(messageForError(err));
    }
  }

  return (
    <PageFrame title="Models" icon={Boxes} onRefresh={() => setTick((value) => value + 1)}>
      <form className="toolbar-form" onSubmit={createModel}>
        <input name="model_public_name" value={createName} onChange={(event) => setCreateName(event.target.value)} placeholder="Public model name" required />
        <button type="submit" className="primary"><Plus size={16} />Create</button>
      </form>
      {message ? <div className="inline-note">{message}</div> : null}
      <ResourceState resource={resource}>
        {({ models, upstreams, mappings }) => {
          const selected = models.find((model) => model.id === selectedId) ?? models[0] ?? null;
          return models.length === 0 ? (
            <EmptyState text="No models configured." />
          ) : (
            <div className="split-workspace">
              <Table
                empty="No models configured."
                columns={["Model", "Visible", "Enabled", "Mappings"]}
                rows={models.map((model) => [
                  <button key="model" type="button" className="link-button" onClick={() => setSelectedId(model.id)}>{model.public_name}</button>,
                  yesNo(model.visible_to_users),
                  yesNo(model.enabled),
                  mappings[model.id]?.length ?? 0
                ])}
              />
              {selected ? (
                <ModelEditor
                  key={selected.id}
                  session={session}
                  model={selected}
                  upstreams={upstreams}
                  mappings={mappings[selected.id] ?? []}
                  onChanged={() => setTick((value) => value + 1)}
                  onMessage={setMessage}
                />
              ) : null}
            </div>
          );
        }}
      </ResourceState>
    </PageFrame>
  );
}

function ModelEditor({
  session,
  model,
  upstreams,
  mappings,
  onChanged,
  onMessage
}: {
  session: Session;
  model: Model;
  upstreams: Upstream[];
  mappings: ModelMapping[];
  onChanged: () => void;
  onMessage: (message: string | null) => void;
}) {
  const [description, setDescription] = useState(model.description ?? "");
  const [enabled, setEnabled] = useState(Boolean(model.enabled));
  const [visible, setVisible] = useState(Boolean(model.visible_to_users));
  const [draft, setDraft] = useState({ upstream_id: upstreams[0]?.id ?? "", upstream_model_name: "", priority: "100", weight: "1" });

  async function saveModel(event: FormEvent) {
    event.preventDefault();
    onMessage(null);
    try {
      await apiFetch(`/api/admin/models/${model.id}`, {
        method: "PATCH",
        token: session.token,
        body: { description, enabled, visible_to_users: visible }
      });
      onChanged();
    } catch (err) {
      onMessage(messageForError(err));
    }
  }

  async function addMapping(event: FormEvent) {
    event.preventDefault();
    onMessage(null);
    try {
      await apiFetch(`/api/admin/models/${model.id}/mappings`, {
        method: "POST",
        token: session.token,
        body: {
          upstream_id: draft.upstream_id,
          upstream_model_name: draft.upstream_model_name,
          enabled: true,
          priority: Number(draft.priority),
          weight: Number(draft.weight)
        }
      });
      setDraft({ ...draft, upstream_model_name: "" });
      onChanged();
    } catch (err) {
      onMessage(messageForError(err));
    }
  }

  return (
    <section className="editor-panel">
      <form className="stack-form" onSubmit={saveModel}>
        <h2>{model.public_name}</h2>
        <textarea name="model_description" value={description} onChange={(event) => setDescription(event.target.value)} placeholder="Description" />
        <div className="check-row">
          <label className="checkline"><input name="model_enabled" type="checkbox" checked={enabled} onChange={(event) => setEnabled(event.target.checked)} />Enabled</label>
          <label className="checkline"><input name="model_visible" type="checkbox" checked={visible} onChange={(event) => setVisible(event.target.checked)} />Visible</label>
        </div>
        <button type="submit" className="primary"><Save size={16} />Save model</button>
      </form>
      <form className="toolbar-form" onSubmit={addMapping}>
        <select name="mapping_upstream_id" value={draft.upstream_id} onChange={(event) => setDraft({ ...draft, upstream_id: event.target.value })} required>
          {upstreams.map((upstream) => <option key={upstream.id} value={upstream.id}>{upstream.name}</option>)}
        </select>
        <input name="mapping_upstream_model_name" value={draft.upstream_model_name} onChange={(event) => setDraft({ ...draft, upstream_model_name: event.target.value })} placeholder="Upstream model" required />
        <NumberInput label="Priority" value={draft.priority} onChange={(value) => setDraft({ ...draft, priority: value })} />
        <NumberInput label="Weight" value={draft.weight} onChange={(value) => setDraft({ ...draft, weight: value })} />
        <button type="submit"><Plus size={16} />Add mapping</button>
      </form>
      <div className="mapping-list">
        {mappings.length === 0 ? <EmptyState text="No mappings for this model." /> : mappings.map((mapping) => (
          <MappingRow key={mapping.id} session={session} mapping={mapping} upstreams={upstreams} onChanged={onChanged} onMessage={onMessage} />
        ))}
      </div>
    </section>
  );
}

function MappingRow({ session, mapping, upstreams, onChanged, onMessage }: { session: Session; mapping: ModelMapping; upstreams: Upstream[]; onChanged: () => void; onMessage: (message: string | null) => void }) {
  const [draft, setDraft] = useState({
    upstream_id: mapping.upstream_id,
    upstream_model_name: mapping.upstream_model_name,
    enabled: Boolean(mapping.enabled),
    priority: String(mapping.priority),
    weight: String(mapping.weight)
  });

  async function save() {
    onMessage(null);
    try {
      await apiFetch(`/api/admin/model-mappings/${mapping.id}`, {
        method: "PATCH",
        token: session.token,
        body: {
          upstream_id: draft.upstream_id,
          upstream_model_name: draft.upstream_model_name,
          enabled: draft.enabled,
          priority: Number(draft.priority),
          weight: Number(draft.weight)
        }
      });
      onChanged();
    } catch (err) {
      onMessage(messageForError(err));
    }
  }

  async function disable() {
    onMessage(null);
    try {
      await apiFetch(`/api/admin/model-mappings/${mapping.id}/disable`, { method: "POST", token: session.token });
      onChanged();
    } catch (err) {
      onMessage(messageForError(err));
    }
  }

  return (
    <div className="mapping-row">
      <select name="mapping_row_upstream_id" value={draft.upstream_id} onChange={(event) => setDraft({ ...draft, upstream_id: event.target.value })}>
        {upstreams.map((upstream) => <option key={upstream.id} value={upstream.id}>{upstream.name}</option>)}
      </select>
      <input name="mapping_row_upstream_model_name" value={draft.upstream_model_name} onChange={(event) => setDraft({ ...draft, upstream_model_name: event.target.value })} />
      <NumberInput label="Priority" value={draft.priority} onChange={(value) => setDraft({ ...draft, priority: value })} />
      <NumberInput label="Weight" value={draft.weight} onChange={(value) => setDraft({ ...draft, weight: value })} />
      <label className="checkline"><input name="mapping_row_enabled" type="checkbox" checked={draft.enabled} onChange={(event) => setDraft({ ...draft, enabled: event.target.checked })} />Enabled</label>
      <button type="button" onClick={save} title="Save"><Save size={15} /></button>
      <button type="button" onClick={disable} disabled={!mapping.enabled} title="Disable"><X size={15} /></button>
    </div>
  );
}

function UsersPage({ session }: { session: Session }) {
  const [tick, setTick] = useState(0);
  const [create, setCreate] = useState({ email: "", password: "", role: "user", display_name: "" });
  const [message, setMessage] = useState<string | null>(null);
  const resource = useResource(async () => {
    const [users, limits] = await Promise.all([
      apiFetch<User[]>("/api/admin/users", { token: session.token }),
      apiFetch<AdminLimitState>("/api/admin/limits", { token: session.token })
    ]);
    return { users, limits };
  }, [session.token, tick]);

  async function createUser(event: FormEvent) {
    event.preventDefault();
    setMessage(null);
    try {
      await apiFetch("/api/admin/users", {
        method: "POST",
        token: session.token,
        body: { ...create, display_name: create.display_name || null }
      });
      setCreate({ email: "", password: "", role: "user", display_name: "" });
      setTick((value) => value + 1);
    } catch (err) {
      setMessage(messageForError(err));
    }
  }

  return (
    <PageFrame title="Users" icon={Users} onRefresh={() => setTick((value) => value + 1)}>
      <form className="toolbar-form" onSubmit={createUser}>
        <input name="user_email" value={create.email} onChange={(event) => setCreate({ ...create, email: event.target.value })} placeholder="Email" type="email" required />
        <input name="user_password" value={create.password} onChange={(event) => setCreate({ ...create, password: event.target.value })} placeholder="Password" type="password" required />
        <select name="user_role" value={create.role} onChange={(event) => setCreate({ ...create, role: event.target.value })}>
          <option value="user">user</option>
          <option value="admin">admin</option>
        </select>
        <input name="user_display_name" value={create.display_name} onChange={(event) => setCreate({ ...create, display_name: event.target.value })} placeholder="Display name" />
        <button type="submit" className="primary"><Plus size={16} />Create</button>
      </form>
      {message ? <div className="inline-note">{message}</div> : null}
      <ResourceState resource={resource}>
        {({ users, limits }) => (
          <div className="user-list">
            {users.length === 0 ? <EmptyState text="No users found." /> : users.map((user) => (
              <UserRow
                key={user.id}
                session={session}
                user={user}
                limits={limits.users.find((limit) => limit.subject_id === user.id)}
                onChanged={() => setTick((value) => value + 1)}
                onMessage={setMessage}
              />
            ))}
          </div>
        )}
      </ResourceState>
    </PageFrame>
  );
}

function UserRow({ session, user, limits, onChanged, onMessage }: { session: Session; user: User; limits: LimitSubjectState | undefined; onChanged: () => void; onMessage: (message: string | null) => void }) {
  const [draft, setDraft] = useState({ role: user.role, status: user.status, display_name: user.display_name ?? "", password: "" });

  async function save() {
    onMessage(null);
    try {
      await apiFetch(`/api/admin/users/${user.id}`, {
        method: "PATCH",
        token: session.token,
        body: { role: draft.role, status: draft.status, display_name: draft.display_name || null }
      });
      onChanged();
    } catch (err) {
      onMessage(messageForError(err));
    }
  }

  async function resetPassword() {
    onMessage(null);
    try {
      await apiFetch(`/api/admin/users/${user.id}/password`, {
        method: "POST",
        token: session.token,
        body: { password: draft.password }
      });
      setDraft({ ...draft, password: "" });
      onMessage("Password reset.");
    } catch (err) {
      onMessage(messageForError(err));
    }
  }

  return (
    <section className="user-row">
      <div>
        <strong>{user.email}</strong>
        <span>{formatDate(user.last_login_at)}</span>
      </div>
      <input name="user_row_display_name" value={draft.display_name} onChange={(event) => setDraft({ ...draft, display_name: event.target.value })} placeholder="Display name" />
      <select name="user_row_role" value={draft.role} onChange={(event) => setDraft({ ...draft, role: event.target.value })}>
        <option value="user">user</option>
        <option value="admin">admin</option>
      </select>
      <select name="user_row_status" value={draft.status} onChange={(event) => setDraft({ ...draft, status: event.target.value })}>
        <option value="active">active</option>
        <option value="disabled">disabled</option>
      </select>
      <button type="button" onClick={save}><Save size={15} />Save</button>
      <input name="user_row_new_password" value={draft.password} onChange={(event) => setDraft({ ...draft, password: event.target.value })} placeholder="New password" type="password" />
      <button type="button" onClick={resetPassword} disabled={draft.password.length < 8}>Reset</button>
      {limits ? (
        <LimitPolicyEditor
          title="User limits"
          policy={limits.policy}
          compact
          onSave={async (body) => {
            await apiFetch(`/api/admin/users/${user.id}/limits`, {
              method: "PATCH",
              token: session.token,
              body
            });
            onChanged();
          }}
        />
      ) : null}
    </section>
  );
}

function SettingsPage({ session }: { session: Session }) {
  const [tick, setTick] = useState(0);
  const resource = useResource(async () => {
    const [settings, limits] = await Promise.all([
      apiFetch<SettingsSummary>("/api/admin/settings", { token: session.token }),
      apiFetch<AdminLimitState>("/api/admin/limits", { token: session.token })
    ]);
    return { settings, limits };
  }, [session.token, tick]);

  return (
    <PageFrame title="Settings" icon={Settings} onRefresh={() => setTick((value) => value + 1)}>
      <ResourceState resource={resource}>
        {({ settings, limits }) => (
          <>
            <div className="stat-grid">
              <Stat label="Service" value={settings.service} />
              <Stat label="Route strategy" value={settings.route_strategy} />
              <Stat label="Public URL" value={settings.public_url} />
              <Stat label="Log level" value={settings.log_level} />
            </div>
            <Table
              empty="No settings returned."
              columns={["Area", "Value"]}
              rows={[
                ["Bind", settings.bind],
                ["Database", `${settings.database.kind} (${settings.database.configured ? "configured" : "default"})`],
                ["Health checks", `${settings.health_checks_enabled ? "enabled" : "disabled"} (${settings.health_check_interval_ms} ms)`],
                ["Request log retention", `${settings.request_log_retention_days || "disabled"} days`],
                ["Daily usage retention", `${settings.daily_usage_retention_days || "disabled"} days`],
                ["Startup retention", settings.retention_run_on_startup ? "enabled" : "disabled"],
                ["Admin email", settings.admin_email_configured ? "configured" : "not configured"],
                ["Bootstrap key", settings.bootstrap_admin_key_configured ? "configured" : "not configured"],
                ["Users", settings.counts.users],
                ["API keys", settings.counts.api_keys],
                ["Upstreams", settings.counts.upstreams],
                ["Models", settings.counts.models],
                ["Request logs", settings.counts.request_logs]
              ]}
            />
            <LimitPolicyEditor
              title="System default limits"
              policy={limits.system}
              onSave={async (body) => {
                await apiFetch<LimitPolicy>("/api/admin/limits/system", {
                  method: "PATCH",
                  token: session.token,
                  body
                });
                setTick((value) => value + 1);
              }}
            />
          </>
        )}
      </ResourceState>
    </PageFrame>
  );
}

function AdminGate({ session, children }: { session: Session; children: ReactNode }) {
  if (isAdmin(session)) return <>{children}</>;
  return (
    <PageFrame title="Admin only" icon={ShieldAlert}>
      <UnauthorizedState message="This page requires an admin account." />
    </PageFrame>
  );
}

function PageFrame({ title, icon: Icon, onRefresh, children }: { title: string; icon: ComponentType<{ size?: number }>; onRefresh?: () => void; children: ReactNode }) {
  return (
    <section className="panel">
      <header className="panel-header">
        <div>
          <h1>{title}</h1>
        </div>
        {onRefresh ? (
          <button type="button" onClick={onRefresh} aria-label={`Refresh ${title}`}>
            <RefreshCw size={18} />
          </button>
        ) : (
          <Icon size={22} />
        )}
      </header>
      {children}
    </section>
  );
}

function ResourceState<T>({ resource, children }: { resource: Resource<T>; children: (data: T) => ReactNode }) {
  if (resource.status === "loading") {
    return <div className="state"><RefreshCw className="spin" size={18} />Loading</div>;
  }
  if (resource.status === "error") {
    if (resource.error instanceof ApiClientError && (resource.error.status === 401 || resource.error.status === 403)) {
      return <UnauthorizedState message={resource.error.message} />;
    }
    return <div className="state error-state">{messageForError(resource.error)}</div>;
  }
  return <>{children(resource.data)}</>;
}

function UnauthorizedState({ message }: { message: string }) {
  return (
    <div className="state error-state">
      <ShieldAlert size={18} />
      {message}
    </div>
  );
}

function EmptyState({ text }: { text: string }) {
  return <div className="state empty-state">{text}</div>;
}

function Table({ columns, rows, empty }: { columns: string[]; rows: ReactNode[][]; empty: string }) {
  if (rows.length === 0) {
    return <EmptyState text={empty} />;
  }
  return (
    <div className="table-shell">
      <table>
        <thead>
          <tr>{columns.map((column) => <th key={column}>{column}</th>)}</tr>
        </thead>
        <tbody>
          {rows.map((row, rowIndex) => (
            <tr key={rowIndex}>
              {row.map((cell, cellIndex) => <td key={cellIndex} data-label={columns[cellIndex]}>{cell}</td>)}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="stat">
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function LimitSummary({ state }: { state: LimitSubjectState | undefined }) {
  if (!state) return null;
  return (
    <div className="stat-grid">
      <Stat label="Request quota" value={limitCell(state.request_quota)} />
      <Stat label="Token budget" value={limitCell(state.token_budget)} />
      <Stat label="Rate limit" value={limitCell(state.rate_limit)} />
      <Stat label="Concurrency" value={state.concurrency.limit === null ? `${state.concurrency.in_flight} live / unlimited` : `${state.concurrency.remaining} left / ${state.concurrency.limit}`} />
    </div>
  );
}

function LimitPolicyEditor({
  title,
  policy,
  compact,
  onSave
}: {
  title: string;
  policy: LimitPolicy;
  compact?: boolean;
  onSave: (body: Record<string, number | LimitPatchPayload>) => Promise<void>;
}) {
  const [draft, setDraft] = useState(() => policyDraft(policy));
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    setDraft(policyDraft(policy));
  }, [policy]);

  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setMessage(null);
    try {
      await onSave(limitPolicyBody(draft, policy.scope));
      setMessage("Limits saved.");
    } catch (err) {
      setMessage(messageForError(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form className={`limit-editor ${compact ? "compact" : ""}`} onSubmit={submit}>
      <h2>{title}</h2>
      <LimitModeInput label="Request quota" mode={draft.request_quota_mode} value={draft.request_quota} allowInherit={policy.scope !== "system"} onMode={(value) => setDraft({ ...draft, request_quota_mode: value })} onValue={(value) => setDraft({ ...draft, request_quota: value })} />
      <NumberInput label="Request window seconds" value={draft.request_window_seconds} onChange={(value) => setDraft({ ...draft, request_window_seconds: value })} />
      <LimitModeInput label="Token budget" mode={draft.token_quota_mode} value={draft.token_quota} allowInherit={policy.scope !== "system"} onMode={(value) => setDraft({ ...draft, token_quota_mode: value })} onValue={(value) => setDraft({ ...draft, token_quota: value })} />
      <NumberInput label="Token window seconds" value={draft.token_window_seconds} onChange={(value) => setDraft({ ...draft, token_window_seconds: value })} />
      <LimitModeInput label="Rate requests" mode={draft.rate_limit_mode} value={draft.rate_limit_requests} allowInherit={policy.scope !== "system"} onMode={(value) => setDraft({ ...draft, rate_limit_mode: value })} onValue={(value) => setDraft({ ...draft, rate_limit_requests: value })} />
      <NumberInput label="Rate window seconds" value={draft.rate_limit_window_seconds} onChange={(value) => setDraft({ ...draft, rate_limit_window_seconds: value })} />
      <LimitModeInput label="Concurrency" mode={draft.concurrency_mode} value={draft.concurrency_limit} allowInherit={policy.scope !== "system"} onMode={(value) => setDraft({ ...draft, concurrency_mode: value })} onValue={(value) => setDraft({ ...draft, concurrency_limit: value })} />
      <button type="submit" className="primary" disabled={busy}>
        <Save size={16} />
        Save limits
      </button>
      {message ? <div className={message.includes("(") ? "inline-error" : "inline-note"}>{message}</div> : null}
    </form>
  );
}

function Badge({ tone, children }: { tone: "good" | "bad" | "neutral"; children: ReactNode }) {
  return <span className={`badge ${tone}`}>{children}</span>;
}

function OneTimeSecret({ value }: { value: string }) {
  const [copied, setCopied] = useState(false);
  async function copy() {
    await navigator.clipboard.writeText(value);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1200);
  }
  return (
    <div className="secret-box">
      <div>
        <strong>Plaintext key</strong>
        <code>{value}</code>
      </div>
      <button type="button" onClick={copy}>
        {copied ? <Check size={16} /> : <Copy size={16} />}
        {copied ? "Copied" : "Copy"}
      </button>
    </div>
  );
}

function NumberInput({ label, value, onChange, name }: { label: string; value: string; onChange: (value: string) => void; name?: string }) {
  return <input name={name ?? fieldName(label)} value={value} onChange={(event) => onChange(event.target.value)} type="number" min="0" aria-label={label} placeholder={label} />;
}

function LimitModeInput({ label, mode, value, allowInherit, onMode, onValue }: { label: string; mode: LimitMode; value: string; allowInherit: boolean; onMode: (value: LimitMode) => void; onValue: (value: string) => void }) {
  const name = fieldName(label);
  return (
    <label className="limit-mode-input">
      {label}
      <select name={`${name}_mode`} value={mode} onChange={(event) => onMode(event.target.value as LimitMode)}>
        {allowInherit ? <option value="inherit">inherit</option> : null}
        <option value="limited">limited</option>
        <option value="unlimited">unlimited</option>
      </select>
      <input name={`${name}_value`} value={value} onChange={(event) => onValue(event.target.value)} type="number" min="0" aria-label={`${label} value`} placeholder="Value" disabled={mode !== "limited"} required={mode === "limited"} />
    </label>
  );
}

function useResource<T>(loader: () => Promise<T>, deps: DependencyList): Resource<T> {
  const [resource, setResource] = useState<Resource<T>>({ status: "loading" });
  useEffect(() => {
    let active = true;
    setResource({ status: "loading" });
    loader()
      .then((data) => active && setResource({ status: "ready", data }))
      .catch((error) => active && setResource({ status: "error", error: error instanceof Error ? error : new Error(String(error)) }));
    return () => {
      active = false;
    };
  }, deps);
  return resource;
}

function readStoredSession(): Session | null {
  const raw = localStorage.getItem(sessionKey);
  if (!raw) return null;
  try {
    return JSON.parse(raw) as Session;
  } catch {
    clearStoredSession();
    return null;
  }
}

function storeSession(session: Session) {
  localStorage.setItem(sessionKey, JSON.stringify(session));
}

function clearStoredSession() {
  localStorage.removeItem(sessionKey);
}

function isAdmin(session: Session) {
  return session.user.role === "admin";
}

function formatDate(value: string | null | undefined) {
  if (!value) return "-";
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

function formatNumber(value: number) {
  return new Intl.NumberFormat().format(value);
}

function statusTone(status: number | null): "good" | "bad" | "neutral" {
  if (!status) return "neutral";
  return status >= 400 ? "bad" : "good";
}

function summarizeUsage(rows: DailyUsage[]) {
  return rows.reduce(
    (totals, row) => ({
      requests: totals.requests + row.request_count,
      tokens: totals.tokens + row.total_tokens,
      latency: totals.latency + row.latency_ms_sum
    }),
    { requests: 0, tokens: 0, latency: 0 }
  );
}

function requestFilterQuery(filters: { user_id: string; key_id: string; model_id: string; upstream_id: string; status: string; from: string; to: string }) {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(filters)) {
    const trimmed = value.trim();
    if (trimmed) {
      params.set(key, trimmed);
    }
  }
  const query = params.toString();
  return query ? `?${query}` : "";
}

function latestErrorSample(value: string | null | undefined) {
  if (!value) return "-";
  try {
    const samples = JSON.parse(value) as Array<{ at?: string; status?: string; error?: string }>;
    const latest = samples.at(-1);
    return latest ? [latest.error, latest.status, formatDate(latest.at)].filter(Boolean).join(" / ") : "-";
  } catch {
    return value;
  }
}

function yesNo(value: number) {
  return value ? "yes" : "no";
}

function limitCell(bucket: { limit: number | null; used: number; remaining: number | null }) {
  if (bucket.limit === null) {
    return `${formatNumber(bucket.used)} used / unlimited`;
  }
  return `${formatNumber(bucket.remaining ?? 0)} left / ${formatNumber(bucket.limit)}`;
}

function policyDraft(policy: LimitPolicy) {
  return {
    request_quota: valueOrBlank(policy.request_quota),
    request_quota_mode: modeOrUnlimited(policy.request_quota_mode),
    request_window_seconds: String(policy.request_window_seconds),
    token_quota: valueOrBlank(policy.token_quota),
    token_quota_mode: modeOrUnlimited(policy.token_quota_mode),
    token_window_seconds: String(policy.token_window_seconds),
    rate_limit_requests: valueOrBlank(policy.rate_limit_requests),
    rate_limit_mode: modeOrUnlimited(policy.rate_limit_mode),
    rate_limit_window_seconds: String(policy.rate_limit_window_seconds),
    concurrency_limit: valueOrBlank(policy.concurrency_limit),
    concurrency_mode: modeOrUnlimited(policy.concurrency_mode)
  };
}

function valueOrBlank(value: number | null) {
  return value === null ? "" : String(value);
}

function fieldName(label: string) {
  return label.toLowerCase().replace(/[^a-z0-9]+/g, "_").replace(/^_|_$/g, "");
}

function modeOrUnlimited(value: string): LimitMode {
  return value === "inherit" || value === "limited" || value === "unlimited" ? value : "unlimited";
}

function limitPolicyBody(draft: ReturnType<typeof policyDraft>, scope: string): Record<string, number | LimitPatchPayload> {
  const body: Record<string, number | LimitPatchPayload> = {
    request_quota: limitPatch(draft.request_quota_mode, draft.request_quota),
    token_quota: limitPatch(draft.token_quota_mode, draft.token_quota),
    rate_limit_requests: limitPatch(draft.rate_limit_mode, draft.rate_limit_requests),
    concurrency_limit: limitPatch(draft.concurrency_mode, draft.concurrency_limit)
  };
  if (scope === "system" || draft.request_quota_mode === "limited") {
    body.request_window_seconds = Number(draft.request_window_seconds || 86400);
  }
  if (scope === "system" || draft.token_quota_mode === "limited") {
    body.token_window_seconds = Number(draft.token_window_seconds || 86400);
  }
  if (scope === "system" || draft.rate_limit_mode === "limited") {
    body.rate_limit_window_seconds = Number(draft.rate_limit_window_seconds || 60);
  }
  return body;
}

function limitPatch(mode: LimitMode, value: string): LimitPatchPayload {
  return mode === "limited" ? { mode, value: Number(value) } : { mode };
}

function messageForError(error: unknown) {
  if (error instanceof ApiClientError) {
    return `${error.message} (${error.code})`;
  }
  if (error instanceof Error) {
    return error.message;
  }
  return "Request failed";
}

function upstreamDefaults() {
  return {
    name: "",
    base_url: "",
    api_key: "",
    enabled: true,
    priority: "100",
    weight: "1",
    timeout_ms: "120000",
    max_retries: "1",
    health_check_path: "/v1/models"
  };
}

function upstreamBody(draft: ReturnType<typeof upstreamDefaults>, editing: boolean) {
  const body: Record<string, unknown> = {
    name: draft.name,
    base_url: draft.base_url,
    enabled: draft.enabled,
    priority: Number(draft.priority),
    weight: Number(draft.weight),
    timeout_ms: Number(draft.timeout_ms),
    max_retries: Number(draft.max_retries),
    health_check_path: draft.health_check_path
  };
  if (!editing || draft.api_key.trim()) {
    body.api_key = draft.api_key;
  }
  return body;
}
