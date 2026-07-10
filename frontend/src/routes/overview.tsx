import { useQuery } from "@tanstack/react-query";
import { Gauge } from "lucide-react";
import { PageFrame } from "../components/layout/page-frame";
import { Badge } from "../components/ui/badge";
import { QueryState } from "../components/ui/query-state";
import { Stat } from "../components/ui/stat";
import { DataTable } from "../components/ui/table";
import { LimitSummary } from "../features/limits/limits";
import { AnalyticsPanels } from "../features/analytics/analytics-panels";
import { UsageChart } from "../features/usage/usage-chart";
import { apiFetch } from "../lib/api/client";
import { formatDate, formatNumber, latestErrorSample, requestDrilldownPath, statusTone, summarizeUsage } from "../lib/format";
import { isAdmin, useSession } from "../lib/auth/session";
import type { AdminLimitState, AnalyticsSnapshot, DailyUsage, GatewayMetrics, Model, OverviewResponse, RequestLog, Upstream, UserLimitState } from "../types/api";

type OverviewData =
  | (OverviewResponse & { metrics: null; limits: UserLimitState; analytics: AnalyticsSnapshot; upstreams: Upstream[]; models: Model[] })
  | { user: null; daily_usage: DailyUsage[]; recent_requests: RequestLog[]; metrics: GatewayMetrics; limits: AdminLimitState; analytics: AnalyticsSnapshot; upstreams: Upstream[]; models: Model[] };

export function OverviewPage() {
  const { session } = useSession();
  if (!session) return null;
  const admin = isAdmin(session);
  const query = useQuery({
    queryKey: ["overview", session.token, session.user.role],
    queryFn: async (): Promise<OverviewData> => {
      if (admin) {
        const [dailyUsage, recentRequests, metrics, limits, analytics, upstreams, models] = await Promise.all([
          apiFetch<DailyUsage[]>("/api/admin/usage/daily", { token: session.token }),
          apiFetch<RequestLog[]>("/api/admin/requests", { token: session.token }),
          apiFetch<GatewayMetrics>("/api/admin/metrics", { token: session.token }),
          apiFetch<AdminLimitState>("/api/admin/limits", { token: session.token }),
          apiFetch<AnalyticsSnapshot>("/api/admin/analytics", { token: session.token }),
          apiFetch<Upstream[]>("/api/admin/upstreams", { token: session.token }),
          apiFetch<Model[]>("/api/admin/models", { token: session.token })
        ]);
        return { user: null, daily_usage: dailyUsage, recent_requests: recentRequests, metrics, limits, analytics, upstreams, models };
      }
      const [overview, limits, analytics, models] = await Promise.all([
        apiFetch<OverviewResponse>("/api/overview", { token: session.token }),
        apiFetch<UserLimitState>("/api/limits", { token: session.token }),
        apiFetch<AnalyticsSnapshot>("/api/analytics", { token: session.token }),
        apiFetch<Model[]>("/api/models", { token: session.token })
      ]);
      return { ...overview, metrics: null, limits, analytics, upstreams: [], models };
    }
  });

  return (
    <PageFrame title="Overview" icon={Gauge} onRefresh={() => void query.refetch()} refreshing={query.isFetching}>
      <QueryState query={query}>
        {(overview) => {
          const totals = summarizeUsage(overview.daily_usage);
          const errors = overview.recent_requests.filter((request) => (request.status_code ?? 500) >= 400).length;
          const failingUpstreams = overview.metrics?.upstream_health.filter((upstream) => upstream.last_health_status === "down" || upstream.last_health_status === "degraded") ?? [];
          const upstreamNames = new Map(overview.upstreams.map((upstream) => [upstream.id, upstream.name]));
          const modelNames = new Map(overview.models.map((model) => [model.id, model.public_name]));
          return (
            <>
              <div className="grid min-w-0 grid-cols-4 gap-3 max-[980px]:grid-cols-2 max-[760px]:grid-cols-1">
                <Stat label="Requests" value={formatNumber(overview.metrics?.request_count ?? totals.requests)} />
                <Stat label="Tokens" value={formatNumber(overview.metrics?.token_usage.total_tokens ?? totals.tokens)} />
                <Stat label="Errors" value={formatNumber(overview.metrics?.error_count ?? errors)} />
                <Stat label="Avg latency" value={overview.metrics?.latency.avg_ms ? `${Math.round(overview.metrics.latency.avg_ms)} ms` : totals.requests ? `${Math.round(totals.latency / totals.requests)} ms` : "-"} />
              </div>
              <LimitSummary state={"user" in overview.limits ? overview.limits.user : overview.limits.users[0]} />
              <AnalyticsPanels analytics={overview.analytics} upstreamNames={upstreamNames} modelNames={modelNames} showUserDrilldowns={admin} />
              <UsageChart rows={overview.daily_usage} />
              {overview.metrics ? (
                <DataTable
                  empty="No unhealthy upstreams."
                  columns={["Upstream", "Health", "Errors", "Last down", "Recent issue"]}
                  rows={failingUpstreams.map((upstream) => [
                    <a key="upstream" className="font-semibold text-emerald-800 hover:text-emerald-950" href={requestDrilldownPath({ upstream_id: upstream.upstream_id, status: "error" })}>{upstream.name}</a>,
                    <Badge key="health" tone="bad">{upstream.last_health_status}</Badge>,
                    formatNumber(upstream.error_count),
                    formatDate(upstream.last_down_at ?? upstream.last_degraded_at),
                    latestErrorSample(upstream.recent_error_samples)
                  ])}
                />
              ) : null}
              <DataTable
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
      </QueryState>
    </PageFrame>
  );
}
