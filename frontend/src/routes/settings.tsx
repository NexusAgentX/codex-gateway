import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Settings } from "lucide-react";
import { PageFrame } from "../components/layout/page-frame";
import { QueryState } from "../components/ui/query-state";
import { Stat } from "../components/ui/stat";
import { DataTable } from "../components/ui/table";
import { LimitPolicyEditor } from "../features/limits/limits";
import { apiFetch } from "../lib/api/client";
import { useSession } from "../lib/auth/session";
import type { AdminLimitState, LimitPolicy, SettingsSummary } from "../types/api";

export function SettingsPage() {
  const { session } = useSession();
  const queryClient = useQueryClient();
  if (!session) return null;
  const queryKey = ["settings", session.token];
  const query = useQuery({
    queryKey,
    queryFn: async () => {
      const [settings, limits] = await Promise.all([
        apiFetch<SettingsSummary>("/api/admin/settings", { token: session.token }),
        apiFetch<AdminLimitState>("/api/admin/limits", { token: session.token })
      ]);
      return { settings, limits };
    }
  });

  return (
    <PageFrame title="Settings" icon={Settings} onRefresh={() => void query.refetch()} refreshing={query.isFetching}>
      <QueryState query={query}>
        {({ settings, limits }) => (
          <>
            <div className="grid min-w-0 grid-cols-4 gap-3 max-[980px]:grid-cols-2 max-[760px]:grid-cols-1">
              <Stat label="Service" value={settings.service} />
              <Stat label="Route strategy" value={settings.route_strategy} />
              <Stat label="Public URL" value={settings.public_url} />
              <Stat label="Log level" value={settings.log_level} />
            </div>
            <DataTable
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
                await queryClient.invalidateQueries({ queryKey });
              }}
            />
          </>
        )}
      </QueryState>
    </PageFrame>
  );
}
