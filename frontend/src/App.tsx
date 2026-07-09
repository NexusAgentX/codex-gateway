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
  type LoginResponse,
  type LoginUser,
  type Model,
  type ModelMapping,
  type OverviewResponse,
  type RequestLog,
  type SettingsSummary,
  type Upstream,
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
          <input value={email} onChange={(event) => setEmail(event.target.value)} autoComplete="username" required />
        </label>
        <label>
          Password
          <input
            value={password}
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
      const [dailyUsage, recentRequests] = await Promise.all([
        apiFetch<DailyUsage[]>("/api/admin/usage/daily", { token: session.token }),
        apiFetch<RequestLog[]>("/api/admin/requests", { token: session.token })
      ]);
      return { user: null, daily_usage: dailyUsage, recent_requests: recentRequests };
    }
    const overview = await apiFetch<OverviewResponse>("/api/overview", { token: session.token });
    return overview;
  }, [session.token, session.user.role, tick]);

  return (
    <PageFrame title="Overview" icon={Gauge} onRefresh={() => setTick((value) => value + 1)}>
      <ResourceState resource={resource}>
        {(overview) => {
          const totals = summarizeUsage(overview.daily_usage);
          const errors = overview.recent_requests.filter((request) => (request.status_code ?? 500) >= 400).length;
          return (
            <>
              <div className="stat-grid">
                <Stat label="Requests" value={String(totals.requests)} />
                <Stat label="Tokens" value={formatNumber(totals.tokens)} />
                <Stat label="Errors" value={String(errors)} />
                <Stat label="Avg latency" value={totals.requests ? `${Math.round(totals.latency / totals.requests)} ms` : "-"} />
              </div>
              <Table
                empty="No recent requests yet."
                columns={["Started", "Status", "Model", "Upstream", "Latency", "Usage"]}
                rows={overview.recent_requests.slice(0, 12).map((request) => [
                  formatDate(request.started_at),
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
    const keys = await apiFetch<ApiKeySummary[]>(admin ? "/api/admin/api-keys" : "/api/api-keys", { token: session.token });
    const users = admin ? await apiFetch<User[]>("/api/admin/users", { token: session.token }) : [];
    return { keys, users };
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
          <select value={userId} onChange={(event) => setUserId(event.target.value)} aria-label="User">
            {resource.status === "ready" && resource.data.users.map((user) => (
              <option key={user.id} value={user.id}>{user.email}</option>
            ))}
          </select>
        ) : null}
        <input value={name} onChange={(event) => setName(event.target.value)} placeholder="Key name" required />
        <input value={expiresAt} onChange={(event) => setExpiresAt(event.target.value)} type="datetime-local" />
        <button type="submit" className="primary" disabled={busy}>
          <Plus size={16} />
          Create
        </button>
      </form>
      {plaintext ? <OneTimeSecret value={plaintext} /> : null}
      {actionError ? <div className="inline-error">{actionError}</div> : null}
      <ResourceState resource={resource}>
        {({ keys, users }) => (
          <Table
            empty="No API keys have been created."
            columns={["Name", "Owner", "Prefix", "Status", "Last used", "Expires", "Actions"]}
            rows={keys.map((key) => [
              key.name,
              users.find((user) => user.id === key.user_id)?.email ?? key.user_id,
              key.key_prefix,
              <Badge key="status" tone={key.status === "active" ? "good" : "bad"}>{key.status}</Badge>,
              formatDate(key.last_used_at),
              formatDate(key.expires_at),
              <div key="actions" className="row-actions">
                <button type="button" onClick={() => setStatus(key.id, "disable")} disabled={busy || key.status !== "active"} title="Disable">
                  <X size={15} />
                </button>
                <button type="button" onClick={() => setStatus(key.id, "revoke")} disabled={busy || key.status === "revoked"} title="Revoke">
                  <Trash2 size={15} />
                </button>
              </div>
            ])}
          />
        )}
      </ResourceState>
    </PageFrame>
  );
}

function RequestsPage({ session }: { session: Session }) {
  const [tick, setTick] = useState(0);
  const admin = isAdmin(session);
  const resource = useResource(async () => {
    const [requests, upstreams, models] = await Promise.all([
      apiFetch<RequestLog[]>(admin ? "/api/admin/requests" : "/api/requests", { token: session.token }),
      admin ? apiFetch<Upstream[]>("/api/admin/upstreams", { token: session.token }) : Promise.resolve([]),
      admin ? apiFetch<Model[]>("/api/admin/models", { token: session.token }) : Promise.resolve([])
    ]);
    return { requests, upstreams, models };
  }, [session.token, admin, tick]);

  return (
    <PageFrame title="Requests" icon={ListChecks} onRefresh={() => setTick((value) => value + 1)}>
      <ResourceState resource={resource}>
        {({ requests, upstreams, models }) => {
          const upstreamNames = new Map(upstreams.map((upstream) => [upstream.id, upstream.name]));
          const modelNames = new Map(models.map((model) => [model.id, model.public_name]));
          return (
            <Table
              empty="No requests have been logged."
              columns={["Started", "Status", "Model", "Upstream", "Latency", "Usage", "Error code"]}
              rows={requests.map((request) => [
                formatDate(request.started_at),
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
        <input value={draft.name} onChange={(event) => setDraft({ ...draft, name: event.target.value })} placeholder="Name" required />
        <input value={draft.base_url} onChange={(event) => setDraft({ ...draft, base_url: event.target.value })} placeholder="Base URL" required />
        <input value={draft.api_key} onChange={(event) => setDraft({ ...draft, api_key: event.target.value })} placeholder={editing ? "New API key optional" : "API key"} required={!editing} />
        <input value={draft.health_check_path} onChange={(event) => setDraft({ ...draft, health_check_path: event.target.value })} placeholder="/v1/models" />
        <NumberInput label="Priority" value={draft.priority} onChange={(value) => setDraft({ ...draft, priority: value })} />
        <NumberInput label="Weight" value={draft.weight} onChange={(value) => setDraft({ ...draft, weight: value })} />
        <NumberInput label="Timeout ms" value={draft.timeout_ms} onChange={(value) => setDraft({ ...draft, timeout_ms: value })} />
        <NumberInput label="Retries" value={draft.max_retries} onChange={(value) => setDraft({ ...draft, max_retries: value })} />
        <label className="checkline">
          <input type="checkbox" checked={draft.enabled} onChange={(event) => setDraft({ ...draft, enabled: event.target.checked })} />
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
            columns={["Name", "Base URL", "Priority", "Weight", "Health", "Timeout", "Actions"]}
            rows={upstreams.map((upstream) => [
              upstream.name,
              upstream.base_url,
              upstream.priority,
              upstream.weight,
              <Badge key="health" tone={upstream.last_health_status === "healthy" ? "good" : upstream.last_health_status === "unknown" ? "neutral" : "bad"}>{upstream.last_health_status}</Badge>,
              `${upstream.timeout_ms} ms`,
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
        <input value={createName} onChange={(event) => setCreateName(event.target.value)} placeholder="Public model name" required />
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
        <textarea value={description} onChange={(event) => setDescription(event.target.value)} placeholder="Description" />
        <div className="check-row">
          <label className="checkline"><input type="checkbox" checked={enabled} onChange={(event) => setEnabled(event.target.checked)} />Enabled</label>
          <label className="checkline"><input type="checkbox" checked={visible} onChange={(event) => setVisible(event.target.checked)} />Visible</label>
        </div>
        <button type="submit" className="primary"><Save size={16} />Save model</button>
      </form>
      <form className="toolbar-form" onSubmit={addMapping}>
        <select value={draft.upstream_id} onChange={(event) => setDraft({ ...draft, upstream_id: event.target.value })} required>
          {upstreams.map((upstream) => <option key={upstream.id} value={upstream.id}>{upstream.name}</option>)}
        </select>
        <input value={draft.upstream_model_name} onChange={(event) => setDraft({ ...draft, upstream_model_name: event.target.value })} placeholder="Upstream model" required />
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
      <select value={draft.upstream_id} onChange={(event) => setDraft({ ...draft, upstream_id: event.target.value })}>
        {upstreams.map((upstream) => <option key={upstream.id} value={upstream.id}>{upstream.name}</option>)}
      </select>
      <input value={draft.upstream_model_name} onChange={(event) => setDraft({ ...draft, upstream_model_name: event.target.value })} />
      <NumberInput label="Priority" value={draft.priority} onChange={(value) => setDraft({ ...draft, priority: value })} />
      <NumberInput label="Weight" value={draft.weight} onChange={(value) => setDraft({ ...draft, weight: value })} />
      <label className="checkline"><input type="checkbox" checked={draft.enabled} onChange={(event) => setDraft({ ...draft, enabled: event.target.checked })} />Enabled</label>
      <button type="button" onClick={save} title="Save"><Save size={15} /></button>
      <button type="button" onClick={disable} disabled={!mapping.enabled} title="Disable"><X size={15} /></button>
    </div>
  );
}

function UsersPage({ session }: { session: Session }) {
  const [tick, setTick] = useState(0);
  const [create, setCreate] = useState({ email: "", password: "", role: "user", display_name: "" });
  const [message, setMessage] = useState<string | null>(null);
  const resource = useResource(() => apiFetch<User[]>("/api/admin/users", { token: session.token }), [session.token, tick]);

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
        <input value={create.email} onChange={(event) => setCreate({ ...create, email: event.target.value })} placeholder="Email" type="email" required />
        <input value={create.password} onChange={(event) => setCreate({ ...create, password: event.target.value })} placeholder="Password" type="password" required />
        <select value={create.role} onChange={(event) => setCreate({ ...create, role: event.target.value })}>
          <option value="user">user</option>
          <option value="admin">admin</option>
        </select>
        <input value={create.display_name} onChange={(event) => setCreate({ ...create, display_name: event.target.value })} placeholder="Display name" />
        <button type="submit" className="primary"><Plus size={16} />Create</button>
      </form>
      {message ? <div className="inline-note">{message}</div> : null}
      <ResourceState resource={resource}>
        {(users) => (
          <div className="user-list">
            {users.length === 0 ? <EmptyState text="No users found." /> : users.map((user) => (
              <UserRow key={user.id} session={session} user={user} onChanged={() => setTick((value) => value + 1)} onMessage={setMessage} />
            ))}
          </div>
        )}
      </ResourceState>
    </PageFrame>
  );
}

function UserRow({ session, user, onChanged, onMessage }: { session: Session; user: User; onChanged: () => void; onMessage: (message: string | null) => void }) {
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
      <input value={draft.display_name} onChange={(event) => setDraft({ ...draft, display_name: event.target.value })} placeholder="Display name" />
      <select value={draft.role} onChange={(event) => setDraft({ ...draft, role: event.target.value })}>
        <option value="user">user</option>
        <option value="admin">admin</option>
      </select>
      <select value={draft.status} onChange={(event) => setDraft({ ...draft, status: event.target.value })}>
        <option value="active">active</option>
        <option value="disabled">disabled</option>
      </select>
      <button type="button" onClick={save}><Save size={15} />Save</button>
      <input value={draft.password} onChange={(event) => setDraft({ ...draft, password: event.target.value })} placeholder="New password" type="password" />
      <button type="button" onClick={resetPassword} disabled={draft.password.length < 8}>Reset</button>
    </section>
  );
}

function SettingsPage({ session }: { session: Session }) {
  const [tick, setTick] = useState(0);
  const resource = useResource(() => apiFetch<SettingsSummary>("/api/admin/settings", { token: session.token }), [session.token, tick]);

  return (
    <PageFrame title="Settings" icon={Settings} onRefresh={() => setTick((value) => value + 1)}>
      <ResourceState resource={resource}>
        {(settings) => (
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
                ["Admin email", settings.admin_email_configured ? "configured" : "not configured"],
                ["Bootstrap key", settings.bootstrap_admin_key_configured ? "configured" : "not configured"],
                ["Users", settings.counts.users],
                ["API keys", settings.counts.api_keys],
                ["Upstreams", settings.counts.upstreams],
                ["Models", settings.counts.models],
                ["Request logs", settings.counts.request_logs]
              ]}
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
              {row.map((cell, cellIndex) => <td key={cellIndex}>{cell}</td>)}
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

function NumberInput({ label, value, onChange }: { label: string; value: string; onChange: (value: string) => void }) {
  return <input value={value} onChange={(event) => onChange(event.target.value)} type="number" min="0" aria-label={label} placeholder={label} />;
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

function yesNo(value: number) {
  return value ? "yes" : "no";
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
