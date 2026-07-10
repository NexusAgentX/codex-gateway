import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Save, Settings } from "lucide-react";
import { useEffect, useState, type FormEvent } from "react";
import { PageFrame } from "../components/layout/page-frame";
import { Button } from "../components/ui/button";
import { Field, Select } from "../components/ui/form";
import { Notice } from "../components/ui/notice";
import { NumberInput } from "../components/ui/number-input";
import { QueryState } from "../components/ui/query-state";
import { Stat } from "../components/ui/stat";
import { DataTable } from "../components/ui/table";
import { LimitPolicyEditor } from "../features/limits/limits";
import { apiFetch } from "../lib/api/client";
import { useSession } from "../lib/auth/session";
import { formatNumber, messageForError } from "../lib/format";
import type { AdminLimitState, LimitPolicy, SettingsDatabaseValues, SettingsSummary } from "../types/api";

type SettingsDraft = {
  route_strategy: string;
  default_request_timeout_ms: string;
  max_request_body_bytes: string;
  request_log_retention_days: string;
  daily_usage_retention_days: string;
  expose_debug_headers: "" | "true" | "false";
};

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
              <Stat label="Body limit" value={`${formatNumber(settings.max_request_body_bytes)} bytes`} />
              <Stat label="Log level" value={settings.log_level} />
            </div>
            <RuntimeSettingsEditor
              settings={settings}
              onSave={async (body) => {
                await apiFetch<SettingsSummary>("/api/admin/settings", {
                  method: "PATCH",
                  token: session.token,
                  body
                });
                await queryClient.invalidateQueries({ queryKey });
              }}
            />
            <DataTable
              empty="No environment values returned."
              columns={["Environment-derived value", "Value", "Restart"]}
              rows={settings.environment.map((item) => [
                item.label,
                formatSettingValue(item.value),
                item.requires_restart ? "required to change" : "not required"
              ])}
            />
            <DataTable
              empty="No settings returned."
              columns={["Area", "Value"]}
              rows={[
                ["Database", `${settings.database.kind} (${settings.database.configured ? "configured" : "default"})`],
                ["Settings precedence", settings.runtime.precedence],
                ["Default timeout", `${formatNumber(settings.default_request_timeout_ms)} ms`],
                ["Request log retention", retentionValue(settings.request_log_retention_days)],
                ["Daily usage retention", retentionValue(settings.daily_usage_retention_days)],
                ["Debug headers", settings.expose_debug_headers ? "enabled" : "disabled"],
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

function RuntimeSettingsEditor({
  settings,
  onSave
}: {
  settings: SettingsSummary;
  onSave: (body: Record<string, string | number | boolean | null>) => Promise<void>;
}) {
  const [draft, setDraft] = useState(() => draftFromSettings(settings.database.settings));
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const shadowed = settings.runtime.fields.filter((field) => field.source === "environment" && field.database_value !== null);
  const restartFields = settings.runtime.fields.filter((field) => field.requires_restart);

  useEffect(() => {
    setDraft(draftFromSettings(settings.database.settings));
  }, [settings]);

  async function submit(event: FormEvent) {
    event.preventDefault();
    setMessage(null);
    const validationError = validateDraft(draft);
    if (validationError) {
      setMessage(validationError);
      return;
    }
    setBusy(true);
    try {
      await onSave(bodyFromDraft(draft));
      setMessage("Settings saved.");
    } catch (err) {
      setMessage(messageForError(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form className="grid min-w-0 gap-3 rounded-lg border border-zinc-200 bg-white p-3" onSubmit={submit}>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h2 className="text-base font-semibold text-zinc-950">Database runtime settings</h2>
        <Button type="submit" variant="primary" disabled={busy}>
          <Save size={16} />
          Save settings
        </Button>
      </div>
      {shadowed.length > 0 ? <Notice tone="note">Some database values are saved but currently shadowed by environment values.</Notice> : null}
      {restartFields.length > 0 ? <Notice tone="note">One or more changes will require a gateway restart.</Notice> : null}
      <div className="grid min-w-0 grid-cols-3 gap-3 max-[980px]:grid-cols-2 max-[760px]:grid-cols-1">
        <Field label="Route strategy">
          <Select value={draft.route_strategy} onChange={(event) => setDraft({ ...draft, route_strategy: event.target.value })}>
            <option value="">use effective default</option>
            <option value="priority">priority</option>
            <option value="weighted">weighted</option>
            <option value="sticky_by_key">sticky_by_key</option>
          </Select>
        </Field>
        <Field label="Default timeout ms">
          <NumberInput label="Default timeout ms" value={draft.default_request_timeout_ms} min="1" onChange={(value) => setDraft({ ...draft, default_request_timeout_ms: value })} />
        </Field>
        <Field label="Max body bytes">
          <NumberInput label="Max body bytes" value={draft.max_request_body_bytes} min="1" onChange={(value) => setDraft({ ...draft, max_request_body_bytes: value })} />
        </Field>
        <Field label="Request log retention days">
          <NumberInput label="Request log retention days" value={draft.request_log_retention_days} onChange={(value) => setDraft({ ...draft, request_log_retention_days: value })} />
        </Field>
        <Field label="Daily usage retention days">
          <NumberInput label="Daily usage retention days" value={draft.daily_usage_retention_days} onChange={(value) => setDraft({ ...draft, daily_usage_retention_days: value })} />
        </Field>
        <Field label="Debug headers">
          <Select value={draft.expose_debug_headers} onChange={(event) => setDraft({ ...draft, expose_debug_headers: event.target.value as SettingsDraft["expose_debug_headers"] })}>
            <option value="">use effective default</option>
            <option value="false">disabled</option>
            <option value="true">enabled</option>
          </Select>
        </Field>
      </div>
      <DataTable
        empty="No runtime settings returned."
        columns={["Setting", "Effective", "Source", "Database"]}
        rows={settings.runtime.fields.map((field) => [
          field.label,
          formatSettingValue(field.value),
          field.source,
          field.database_value === null ? "unset" : formatSettingValue(field.database_value)
        ])}
      />
      {message ? <Notice tone={message === "Settings saved." ? "note" : "error"}>{message}</Notice> : null}
    </form>
  );
}

function draftFromSettings(settings: SettingsDatabaseValues): SettingsDraft {
  return {
    route_strategy: settings.route_strategy ?? "",
    default_request_timeout_ms: valueOrBlank(settings.default_request_timeout_ms),
    max_request_body_bytes: valueOrBlank(settings.max_request_body_bytes),
    request_log_retention_days: valueOrBlank(settings.request_log_retention_days),
    daily_usage_retention_days: valueOrBlank(settings.daily_usage_retention_days),
    expose_debug_headers: settings.expose_debug_headers === null ? "" : String(settings.expose_debug_headers) as "true" | "false"
  };
}

function bodyFromDraft(draft: SettingsDraft) {
  return {
    route_strategy: draft.route_strategy || null,
    default_request_timeout_ms: nullableNumber(draft.default_request_timeout_ms),
    max_request_body_bytes: nullableNumber(draft.max_request_body_bytes),
    request_log_retention_days: nullableNumber(draft.request_log_retention_days),
    daily_usage_retention_days: nullableNumber(draft.daily_usage_retention_days),
    expose_debug_headers: draft.expose_debug_headers === "" ? null : draft.expose_debug_headers === "true"
  };
}

function validateDraft(draft: SettingsDraft) {
  return (
    positiveNumber("Default timeout ms", draft.default_request_timeout_ms) ??
    positiveNumber("Max body bytes", draft.max_request_body_bytes) ??
    nonNegativeNumber("Request log retention days", draft.request_log_retention_days) ??
    nonNegativeNumber("Daily usage retention days", draft.daily_usage_retention_days)
  );
}

function positiveNumber(label: string, value: string) {
  if (value.trim() === "") return null;
  const parsed = Number(value);
  return Number.isInteger(parsed) && parsed >= 1 ? null : `${label} must be an integer of at least 1.`;
}

function nonNegativeNumber(label: string, value: string) {
  if (value.trim() === "") return null;
  const parsed = Number(value);
  return Number.isInteger(parsed) && parsed >= 0 ? null : `${label} must be zero or greater.`;
}

function nullableNumber(value: string) {
  return value.trim() === "" ? null : Number(value);
}

function valueOrBlank(value: number | null) {
  return value === null ? "" : String(value);
}

function retentionValue(days: number) {
  return days === 0 ? "disabled" : `${formatNumber(days)} days`;
}

function formatSettingValue(value: string | number | boolean | null) {
  if (value === null) return "unset";
  if (typeof value === "boolean") return value ? "enabled" : "disabled";
  if (typeof value === "number") return formatNumber(value);
  return value;
}
