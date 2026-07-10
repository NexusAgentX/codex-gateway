import { useQuery } from "@tanstack/react-query";
import { BarChart3 } from "lucide-react";
import { PageFrame } from "../components/layout/page-frame";
import { Badge } from "../components/ui/badge";
import { QueryState } from "../components/ui/query-state";
import { DataTable } from "../components/ui/table";
import { AnalyticsPanels } from "../features/analytics/analytics-panels";
import { LimitSummary } from "../features/limits/limits";
import { UsageChart } from "../features/usage/usage-chart";
import { UsageSummaryStats } from "../features/usage/usage-summary";
import { apiFetch } from "../lib/api/client";
import { formatDate, formatNumber, formatPercent, requestDrilldownPath, statusTone, yesNo } from "../lib/format";
import { isAdmin, useSession } from "../lib/auth/session";
import type { AdminLimitState, AnalyticsSnapshot, DailyUsage, Model, Upstream, UsageSummary, UserLimitState } from "../types/api";

export function UsagePage() {
  const { session } = useSession();
  if (!session) return null;
  const admin = isAdmin(session);
  const query = useQuery({
    queryKey: ["usage", session.token, session.user.role],
    queryFn: async () => {
      const [summary, dailyUsage, models, limits, analytics, upstreams] = await Promise.all([
        apiFetch<UsageSummary>(admin ? "/api/admin/usage/summary" : "/api/usage/summary", { token: session.token }),
        apiFetch<DailyUsage[]>(admin ? "/api/admin/usage/daily" : "/api/usage/daily", { token: session.token }),
        apiFetch<Model[]>(admin ? "/api/admin/models" : "/api/models", { token: session.token }),
        apiFetch<UserLimitState | AdminLimitState>(admin ? "/api/admin/limits" : "/api/limits", { token: session.token }),
        apiFetch<AnalyticsSnapshot>(admin ? "/api/admin/analytics" : "/api/analytics", { token: session.token }),
        admin ? apiFetch<Upstream[]>("/api/admin/upstreams", { token: session.token }) : Promise.resolve([])
      ]);
      return { summary, dailyUsage, models, limits, analytics, upstreams };
    }
  });

  return (
    <PageFrame title="Usage" icon={BarChart3} onRefresh={() => void query.refetch()} refreshing={query.isFetching}>
      <QueryState query={query}>
        {({ summary, dailyUsage, models, limits, analytics, upstreams }) => {
          const modelNames = new Map(models.map((model) => [model.id, model.public_name]));
          const upstreamNames = new Map(upstreams.map((upstream) => [upstream.id, upstream.name]));
          return (
            <>
              <UsageSummaryStats summary={summary} />
              <LimitSummary state={"user" in limits ? limits.user : limits.users[0]} />
              <AnalyticsPanels analytics={analytics} modelNames={modelNames} upstreamNames={upstreamNames} showUserDrilldowns={admin} />
              <UsageChart rows={dailyUsage} />
              <DataTable
                empty="No models are currently available."
                columns={["Model", "Description", "Enabled", "Visible"]}
                rows={models.map((model) => [
                  model.public_name,
                  model.description ?? "-",
                  yesNo(model.enabled),
                  yesNo(model.visible_to_users)
                ])}
              />
              <DataTable
                empty="No usage has been recorded."
                columns={["Date", "Key", "Model", "Upstream", "Requests", "Tokens", "Errors"]}
                rows={dailyUsage.map((row) => [
                  row.date,
                  row.api_key_id,
                  row.model_id ?? "-",
                  row.upstream_id ?? "-",
                  formatNumber(row.request_count),
                  formatNumber(row.total_tokens),
                  `${formatNumber(row.error_count)} (${formatPercent(row.request_count ? row.error_count / row.request_count : 0)})`
                ])}
              />
              <DataTable
                empty="No recent failures."
                columns={["Started", "Request ID", "Status", "Error", "Model", "Upstream"]}
                rows={summary.recent_failures.map((request) => [
                  formatDate(request.started_at),
                  request.request_id,
                  <Badge key="status" tone={statusTone(request.status_code)}>{request.status_code ?? "pending"}</Badge>,
                  <a key="error" className="font-semibold text-emerald-800 hover:text-emerald-950" href={requestDrilldownPath({ status: "error", model_id: request.model_id, upstream_id: request.upstream_id })}>{request.error_code ?? "failed"}</a>,
                  request.model_id ?? "-",
                  request.upstream_id ?? "-"
                ])}
              />
            </>
          );
        }}
      </QueryState>
    </PageFrame>
  );
}
