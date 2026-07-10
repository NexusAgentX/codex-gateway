import { Bar, BarChart, CartesianGrid, Cell, Legend, Line, LineChart, Pie, PieChart, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import { Link } from "react-router-dom";
import type { ReactNode } from "react";
import type { AnalyticsSnapshot } from "../../types/api";
import { formatNumber, formatPercent, requestDrilldownPath } from "../../lib/format";

const chartColors = ["#047857", "#2563eb", "#7c3aed", "#c2410c", "#be123c", "#0f766e", "#52525b"];

export function AnalyticsPanels({
  analytics,
  modelNames = new Map(),
  upstreamNames = new Map(),
  showUserDrilldowns = false
}: {
  analytics: AnalyticsSnapshot;
  modelNames?: Map<string, string>;
  upstreamNames?: Map<string, string>;
  showUserDrilldowns?: boolean;
}) {
  return (
    <div className="grid min-w-0 grid-cols-2 gap-3 max-[980px]:grid-cols-1">
      <RequestVolumeChart rows={analytics.requests_24h} />
      <TokenUsageChart rows={analytics.token_usage_7d} />
      <ModelShareChart rows={analytics.model_share} modelNames={modelNames} />
      <UpstreamErrorChart rows={analytics.upstream_error_rate} upstreamNames={upstreamNames} />
      <LatencyTrendChart rows={analytics.latency_trend} />
      <LatencyBucketChart rows={analytics.latency_buckets} />
      <LatencyDrilldownPanel analytics={analytics} />
      <FailureDrilldownPanel analytics={analytics} modelNames={modelNames} upstreamNames={upstreamNames} showUserDrilldowns={showUserDrilldowns} />
    </div>
  );
}

function ChartShell({ title, meta, empty, children }: { title: string; meta?: string; empty: boolean; children: ReactNode }) {
  return (
    <section className="h-72 min-w-0 rounded-lg border border-zinc-200 bg-white p-3">
      <div className="mb-2 flex min-w-0 items-center justify-between gap-3">
        <h2 className="min-w-0 truncate text-sm font-semibold text-zinc-950">{title}</h2>
        {meta ? <span className="shrink-0 text-xs text-zinc-500">{meta}</span> : null}
      </div>
      {empty ? (
        <div className="grid h-[calc(100%-28px)] place-items-center rounded border border-dashed border-zinc-200 text-sm text-zinc-500">
          No data
        </div>
      ) : (
        <ResponsiveContainer width="100%" height="88%">{children}</ResponsiveContainer>
      )}
    </section>
  );
}

function RequestVolumeChart({ rows }: { rows: AnalyticsSnapshot["requests_24h"] }) {
  const data = rows.map((row) => ({
    ...row,
    label: hourLabel(row.bucket),
    path: requestDrilldownPath({ from: row.bucket, to: hourEnd(row.bucket) }),
    errorPath: requestDrilldownPath({ from: row.bucket, to: hourEnd(row.bucket), status: "error" })
  }));
  return (
    <ChartShell title="24h request volume" meta={`${formatNumber(sum(rows, "request_count"))} requests`} empty={data.length === 0}>
      <BarChart data={data} margin={{ left: 0, right: 8, top: 6, bottom: 0 }}>
        <CartesianGrid stroke="#e4e4e7" strokeDasharray="3 3" vertical={false} />
        <XAxis dataKey="label" tick={{ fontSize: 11, fill: "#71717a" }} tickLine={false} axisLine={false} minTickGap={18} />
        <YAxis tick={{ fontSize: 11, fill: "#71717a" }} tickLine={false} axisLine={false} width={42} />
        <Tooltip />
        <Legend />
        <Bar dataKey="request_count" name="Requests" fill="#047857" onClick={(row) => navigateToPayloadPath(row, "path")} className="cursor-pointer" />
        <Bar dataKey="error_count" name="Errors" fill="#dc2626" onClick={(row) => navigateToPayloadPath(row, "errorPath")} className="cursor-pointer" />
      </BarChart>
    </ChartShell>
  );
}

function TokenUsageChart({ rows }: { rows: AnalyticsSnapshot["token_usage_7d"] }) {
  const data = rows.map((row) => ({ ...row, label: row.date.slice(5), path: requestDrilldownPath({ from: row.date, to: row.date }) }));
  return (
    <ChartShell title="7d token usage" meta={`${formatNumber(sum(rows, "total_tokens"))} tokens`} empty={data.length === 0}>
      <BarChart data={data} margin={{ left: 0, right: 8, top: 6, bottom: 0 }}>
        <CartesianGrid stroke="#e4e4e7" strokeDasharray="3 3" vertical={false} />
        <XAxis dataKey="label" tick={{ fontSize: 11, fill: "#71717a" }} tickLine={false} axisLine={false} />
        <YAxis tick={{ fontSize: 11, fill: "#71717a" }} tickLine={false} axisLine={false} width={52} />
        <Tooltip />
        <Legend />
        <Bar dataKey="prompt_tokens" name="Prompt" stackId="tokens" fill="#2563eb" onClick={(row) => navigateToPayloadPath(row, "path")} className="cursor-pointer" />
        <Bar dataKey="completion_tokens" name="Completion" stackId="tokens" fill="#047857" onClick={(row) => navigateToPayloadPath(row, "path")} className="cursor-pointer" />
      </BarChart>
    </ChartShell>
  );
}

function ModelShareChart({ rows, modelNames }: { rows: AnalyticsSnapshot["model_share"]; modelNames: Map<string, string> }) {
  const data = rows.map((row, index) => ({
    ...row,
    name: row.id ? modelNames.get(row.id) ?? row.id : "Unassigned",
    color: chartColors[index % chartColors.length],
    path: row.id ? requestDrilldownPath({ model_id: row.id }) : requestDrilldownPath({}),
    errorPath: row.id ? requestDrilldownPath({ model_id: row.id, status: "error" }) : requestDrilldownPath({ status: "error" })
  }));
  return (
    <section className="grid h-72 min-w-0 grid-cols-[minmax(0,1fr)_minmax(170px,0.8fr)] gap-2 rounded-lg border border-zinc-200 bg-white p-3 max-[560px]:h-auto max-[560px]:grid-cols-1">
      <div className="min-w-0">
        <div className="mb-2 flex items-center justify-between gap-3">
          <h2 className="truncate text-sm font-semibold text-zinc-950">Model request share</h2>
          <span className="shrink-0 text-xs text-zinc-500">{rows.length} models</span>
        </div>
        {data.length === 0 ? (
          <div className="grid h-[228px] place-items-center rounded border border-dashed border-zinc-200 text-sm text-zinc-500">No data</div>
        ) : (
          <ResponsiveContainer width="100%" height={228}>
            <PieChart>
              <Pie data={data} dataKey="request_count" nameKey="name" innerRadius="48%" outerRadius="78%" paddingAngle={1}>
                {data.map((entry) => <Cell key={entry.name} fill={entry.color} />)}
              </Pie>
              <Tooltip />
            </PieChart>
          </ResponsiveContainer>
        )}
      </div>
      <div className="grid content-start gap-2 overflow-auto pr-1">
        {data.slice(0, 8).map((row) => (
          <Link key={row.name} className="grid min-w-0 grid-cols-[10px_minmax(0,1fr)_auto] items-center gap-2 rounded border border-zinc-200 px-2 py-1.5 text-xs text-zinc-700 hover:bg-zinc-50" to={row.path}>
            <span className="h-2.5 w-2.5 rounded-sm" style={{ backgroundColor: row.color }} />
            <span className="truncate">{row.name}</span>
            <span>{formatPercent(row.share)}</span>
          </Link>
        ))}
        {data.filter((row) => row.error_count > 0).slice(0, 3).map((row) => (
          <Link key={`${row.name}-errors`} className="grid min-w-0 grid-cols-[10px_minmax(0,1fr)_auto] items-center gap-2 rounded border border-red-200 bg-red-50 px-2 py-1.5 text-xs text-red-800 hover:bg-red-100" to={row.errorPath}>
            <span className="h-2.5 w-2.5 rounded-sm bg-red-600" />
            <span className="truncate">{row.name} errors</span>
            <span>{formatNumber(row.error_count)}</span>
          </Link>
        ))}
      </div>
    </section>
  );
}

function UpstreamErrorChart({ rows, upstreamNames }: { rows: AnalyticsSnapshot["upstream_error_rate"]; upstreamNames: Map<string, string> }) {
  const data = rows.map((row) => ({
    ...row,
    label: row.upstream_id ? upstreamNames.get(row.upstream_id) ?? row.upstream_id : "Unassigned",
    rate_percent: row.error_rate * 100,
    path: row.upstream_id ? requestDrilldownPath({ upstream_id: row.upstream_id, status: "error" }) : requestDrilldownPath({ status: "error" })
  }));
  return (
    <ChartShell title="Upstream error rate" meta={`${formatNumber(sum(rows, "error_count"))} errors`} empty={data.length === 0}>
      <BarChart data={data} layout="vertical" margin={{ left: 8, right: 22, top: 6, bottom: 0 }}>
        <CartesianGrid stroke="#e4e4e7" strokeDasharray="3 3" horizontal={false} />
        <XAxis type="number" tick={{ fontSize: 11, fill: "#71717a" }} tickLine={false} axisLine={false} unit="%" />
        <YAxis type="category" dataKey="label" tick={{ fontSize: 11, fill: "#71717a" }} tickLine={false} axisLine={false} width={96} />
        <Tooltip formatter={(value) => `${Number(value).toFixed(1)}%`} />
        <Bar dataKey="rate_percent" name="Error rate" fill="#dc2626" onClick={(row) => navigateToPayloadPath(row, "path")} className="cursor-pointer" />
      </BarChart>
    </ChartShell>
  );
}

function LatencyTrendChart({ rows }: { rows: AnalyticsSnapshot["latency_trend"] }) {
  const data = rows.map((row) => ({ ...row, label: hourLabel(row.bucket), avg_latency_ms: row.avg_latency_ms ?? 0, path: requestDrilldownPath({ from: row.bucket, to: hourEnd(row.bucket) }), errorPath: requestDrilldownPath({ from: row.bucket, to: hourEnd(row.bucket), status: "error" }) }));
  return (
    <ChartShell title="Latency trend" meta={`${formatNumber(Math.round(avg(data.map((row) => row.avg_latency_ms))))} ms avg`} empty={data.length === 0}>
      <LineChart data={data} margin={{ left: 0, right: 8, top: 6, bottom: 0 }}>
        <CartesianGrid stroke="#e4e4e7" strokeDasharray="3 3" vertical={false} />
        <XAxis dataKey="label" tick={{ fontSize: 11, fill: "#71717a" }} tickLine={false} axisLine={false} minTickGap={18} />
        <YAxis tick={{ fontSize: 11, fill: "#71717a" }} tickLine={false} axisLine={false} width={52} />
        <Tooltip />
        <Line type="monotone" dataKey="avg_latency_ms" name="Avg latency" stroke="#7c3aed" strokeWidth={2} dot={{ r: 3 }} activeDot={{ r: 5 }} onClick={(row: unknown) => navigateToPayloadPath(row, "path")} />
      </LineChart>
    </ChartShell>
  );
}

function LatencyBucketChart({ rows }: { rows: AnalyticsSnapshot["latency_buckets"] }) {
  const data = rows.map((row) => ({
    ...row,
    path: requestDrilldownPath({ latency_min_ms: row.min_ms, latency_max_ms: row.max_ms }),
    errorPath: requestDrilldownPath({ latency_min_ms: row.min_ms, latency_max_ms: row.max_ms, status: "error" })
  }));
  return (
    <ChartShell title="Latency buckets" meta={`${formatNumber(sum(rows, "request_count"))} samples`} empty={data.length === 0}>
      <BarChart data={data} margin={{ left: 0, right: 8, top: 6, bottom: 0 }}>
        <CartesianGrid stroke="#e4e4e7" strokeDasharray="3 3" vertical={false} />
        <XAxis dataKey="label" tick={{ fontSize: 11, fill: "#71717a" }} tickLine={false} axisLine={false} />
        <YAxis tick={{ fontSize: 11, fill: "#71717a" }} tickLine={false} axisLine={false} width={42} />
        <Tooltip />
        <Bar dataKey="request_count" name="Requests" fill="#0f766e" onClick={(row) => navigateToPayloadPath(row, "path")} className="cursor-pointer" />
        <Bar dataKey="error_count" name="Errors" fill="#dc2626" onClick={(row) => navigateToPayloadPath(row, "errorPath")} className="cursor-pointer" />
      </BarChart>
    </ChartShell>
  );
}

function LatencyDrilldownPanel({ analytics }: { analytics: AnalyticsSnapshot }) {
  const slowestHours = [...analytics.latency_trend]
    .filter((row) => row.avg_latency_ms !== null)
    .sort((a, b) => (b.avg_latency_ms ?? 0) - (a.avg_latency_ms ?? 0))
    .slice(0, 3);
  const populatedBuckets = analytics.latency_buckets.filter((row) => row.request_count > 0).slice(0, 5);
  const empty = slowestHours.length === 0 && populatedBuckets.length === 0;
  return (
    <section className="min-w-0 rounded-lg border border-zinc-200 bg-white p-3">
      <div className="mb-2 flex min-w-0 items-center justify-between gap-3">
        <h2 className="truncate text-sm font-semibold text-zinc-950">Latency drilldowns</h2>
        <span className="shrink-0 text-xs text-zinc-500">Requests view</span>
      </div>
      {empty ? (
        <div className="grid min-h-32 place-items-center rounded border border-dashed border-zinc-200 text-sm text-zinc-500">No latency samples</div>
      ) : (
        <div className="grid gap-2">
          {slowestHours.map((row) => (
            <FailureLink key={`latency-hour-${row.bucket}`} label={`Hour ${hourLabel(row.bucket)}`} detail={`${formatNumber(Math.round(row.avg_latency_ms ?? 0))} ms avg`} to={requestDrilldownPath({ from: row.bucket, to: hourEnd(row.bucket) })} />
          ))}
          {populatedBuckets.map((row) => (
            <FailureLink key={`latency-bucket-${row.label}`} label={`Bucket ${row.label}`} detail={`${formatNumber(row.request_count)} requests`} to={requestDrilldownPath({ latency_min_ms: row.min_ms, latency_max_ms: row.max_ms })} />
          ))}
          {populatedBuckets.filter((row) => row.error_count > 0).map((row) => (
            <FailureLink key={`latency-error-${row.label}`} label={`${row.label} failures`} detail={`${formatNumber(row.error_count)} errors`} to={requestDrilldownPath({ latency_min_ms: row.min_ms, latency_max_ms: row.max_ms, status: "error" })} />
          ))}
        </div>
      )}
    </section>
  );
}

function FailureDrilldownPanel({
  analytics,
  modelNames,
  upstreamNames,
  showUserDrilldowns
}: {
  analytics: AnalyticsSnapshot;
  modelNames: Map<string, string>;
  upstreamNames: Map<string, string>;
  showUserDrilldowns: boolean;
}) {
  const upstreams = analytics.upstream_error_rate.filter((row) => row.error_count > 0).slice(0, 4);
  const models = analytics.model_share.filter((row) => row.error_count > 0).slice(0, 4);
  const users = showUserDrilldowns ? analytics.user_error_rate.filter((row) => row.error_count > 0).slice(0, 4) : [];
  const empty = upstreams.length === 0 && models.length === 0 && users.length === 0;
  return (
    <section className="min-w-0 rounded-lg border border-zinc-200 bg-white p-3">
      <div className="mb-2 flex min-w-0 items-center justify-between gap-3">
        <h2 className="truncate text-sm font-semibold text-zinc-950">Failure drilldowns</h2>
        <Link className="shrink-0 text-xs font-semibold text-emerald-800 hover:text-emerald-950" to={requestDrilldownPath({ status: "error" })}>All failed</Link>
      </div>
      {empty ? (
        <div className="grid min-h-32 place-items-center rounded border border-dashed border-zinc-200 text-sm text-zinc-500">No failures</div>
      ) : (
        <div className="grid gap-2">
          {upstreams.map((row) => (
            <FailureLink key={`upstream-${row.upstream_id ?? "none"}`} label={row.upstream_id ? upstreamNames.get(row.upstream_id) ?? row.upstream_id : "Unassigned upstream"} detail={`${formatNumber(row.error_count)} errors / ${formatPercent(row.error_rate)}`} to={requestDrilldownPath({ upstream_id: row.upstream_id, status: "error" })} />
          ))}
          {models.map((row) => (
            <FailureLink key={`model-${row.id ?? "none"}`} label={row.id ? modelNames.get(row.id) ?? row.id : "Unassigned model"} detail={`${formatNumber(row.error_count)} model errors`} to={requestDrilldownPath({ model_id: row.id, status: "error" })} />
          ))}
          {users.map((row) => (
            <FailureLink key={`user-${row.user_id}`} label={`User ${row.user_id}`} detail={`${formatNumber(row.error_count)} user errors`} to={requestDrilldownPath({ user_id: row.user_id, status: "error" })} />
          ))}
        </div>
      )}
    </section>
  );
}

function FailureLink({ label, detail, to }: { label: string; detail: string; to: string }) {
  return (
    <Link className="grid min-w-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-2 rounded border border-red-200 bg-red-50 px-2.5 py-2 text-sm text-red-900 hover:bg-red-100" to={to}>
      <span className="truncate font-medium">{label}</span>
      <span className="text-xs text-red-700">{detail}</span>
    </Link>
  );
}

function hourLabel(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value.slice(11, 16) : date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function navigateTo(path: unknown) {
  if (typeof path === "string" && path) {
    window.location.assign(path);
  }
}

function navigateToPayloadPath(row: unknown, key: "path" | "errorPath") {
  if (!row || typeof row !== "object") return;
  const payload = row as Record<string, unknown> & { payload?: Record<string, unknown> };
  const direct = payload[key];
  const nested = payload.payload?.[key];
  navigateTo(direct ?? nested);
}

function hourEnd(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  date.setMinutes(59, 59, 999);
  return date.toISOString();
}

function sum<T, K extends keyof T>(rows: T[], key: K) {
  return rows.reduce((total, row) => total + Number(row[key] ?? 0), 0);
}

function avg(values: number[]) {
  const usable = values.filter((value) => Number.isFinite(value));
  return usable.length ? usable.reduce((total, value) => total + value, 0) / usable.length : 0;
}
