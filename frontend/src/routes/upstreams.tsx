import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Activity, Save, Server, X } from "lucide-react";
import { useState, type FormEvent } from "react";
import { PageFrame } from "../components/layout/page-frame";
import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import { CheckboxLine, Input, Select } from "../components/ui/form";
import { Notice } from "../components/ui/notice";
import { NumberInput } from "../components/ui/number-input";
import { QueryState } from "../components/ui/query-state";
import { DataTable } from "../components/ui/table";
import { apiFetch } from "../lib/api/client";
import { formatDate, latestErrorSample, messageForError } from "../lib/format";
import { useSession } from "../lib/auth/session";
import type { Upstream } from "../types/api";

type UpstreamDraft = {
  name: string;
  base_url: string;
  api_key: string;
  enabled: boolean;
  priority: string;
  weight: string;
  timeout_mode: "default" | "explicit";
  timeout_ms: string;
  max_retries: string;
  health_check_path: string;
};

export function UpstreamsPage() {
  const { session } = useSession();
  const queryClient = useQueryClient();
  const [draft, setDraft] = useState(upstreamDefaults());
  const [editing, setEditing] = useState<Upstream | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  if (!session) return null;
  const queryKey = ["upstreams", session.token];
  const query = useQuery({
    queryKey,
    queryFn: () => apiFetch<Upstream[]>("/api/admin/upstreams", { token: session.token })
  });
  const invalidate = () => queryClient.invalidateQueries({ queryKey });
  const saveMutation = useMutation({
    mutationFn: () =>
      apiFetch(editing ? `/api/admin/upstreams/${editing.id}` : "/api/admin/upstreams", {
        method: editing ? "PATCH" : "POST",
        token: session.token,
        body: upstreamBody(draft, Boolean(editing))
      }),
    onSuccess() {
      setDraft(upstreamDefaults());
      setEditing(null);
      setMessage(null);
      void invalidate();
    },
    onError(error) {
      setMessage(messageForError(error));
    }
  });
  const actionMutation = useMutation({
    mutationFn: ({ id, action }: { id: string; action: "health" | "disable" }) =>
      apiFetch(`/api/admin/upstreams/${id}/${action}`, { method: "POST", token: session.token }),
    onSuccess(_, vars) {
      setMessage(vars.action === "health" ? "Health check completed." : null);
      void invalidate();
    },
    onError(error) {
      setMessage(messageForError(error));
      void invalidate();
    }
  });

  function edit(upstream: Upstream) {
    setEditing(upstream);
    setDraft({
      name: upstream.name,
      base_url: upstream.base_url,
      api_key: "",
      enabled: Boolean(upstream.enabled),
      priority: String(upstream.priority),
      weight: String(upstream.weight),
      timeout_mode: upstream.timeout_ms_is_explicit ? "explicit" : "default",
      timeout_ms: String(upstream.timeout_ms),
      max_retries: String(upstream.max_retries),
      health_check_path: upstream.health_check_path
    });
  }

  function submit(event: FormEvent) {
    event.preventDefault();
    setMessage(null);
    saveMutation.mutate();
  }

  return (
    <PageFrame title="Upstreams" icon={Server} onRefresh={() => void query.refetch()} refreshing={query.isFetching}>
      <form className="grid min-w-0 grid-cols-4 gap-2 max-[980px]:grid-cols-2 max-[760px]:grid-cols-1" onSubmit={submit}>
        <Input name="upstream_name" value={draft.name} onChange={(event) => setDraft({ ...draft, name: event.target.value })} placeholder="Name" required />
        <Input name="upstream_base_url" value={draft.base_url} onChange={(event) => setDraft({ ...draft, base_url: event.target.value })} placeholder="Base URL" required />
        <Input name="upstream_api_key" value={draft.api_key} onChange={(event) => setDraft({ ...draft, api_key: event.target.value })} placeholder={editing ? "New API key optional" : "API key"} required={!editing} />
        <Input name="upstream_health_check_path" value={draft.health_check_path} onChange={(event) => setDraft({ ...draft, health_check_path: event.target.value })} placeholder="/v1/models" />
        <NumberInput label="Priority" value={draft.priority} onChange={(value) => setDraft({ ...draft, priority: value })} />
        <NumberInput label="Weight" value={draft.weight} onChange={(value) => setDraft({ ...draft, weight: value })} />
        <Select name="upstream_timeout_mode" value={draft.timeout_mode} onChange={(event) => setDraft({ ...draft, timeout_mode: event.target.value as "default" | "explicit" })}>
          <option value="default">runtime timeout</option>
          <option value="explicit">explicit timeout</option>
        </Select>
        <NumberInput label="Timeout ms" value={draft.timeout_ms} onChange={(value) => setDraft({ ...draft, timeout_ms: value })} disabled={draft.timeout_mode !== "explicit"} min="1" />
        <NumberInput label="Retries" value={draft.max_retries} onChange={(value) => setDraft({ ...draft, max_retries: value })} />
        <CheckboxLine>
          <input name="upstream_enabled" className="h-4 w-4" type="checkbox" checked={draft.enabled} onChange={(event) => setDraft({ ...draft, enabled: event.target.checked })} />
          Enabled
        </CheckboxLine>
        <Button type="submit" variant="primary" disabled={saveMutation.isPending}>
          <Save size={16} />
          {editing ? "Save" : "Create"}
        </Button>
        {editing ? <Button type="button" onClick={() => { setEditing(null); setDraft(upstreamDefaults()); }}>Cancel</Button> : null}
      </form>
      {message ? <Notice tone={message.includes("(") ? "error" : "note"}>{message}</Notice> : null}
      <QueryState query={query}>
        {(upstreams) => (
          <DataTable
            empty="No upstreams configured."
            columns={["Name", "Base URL", "Priority", "Health", "Last checked", "Recent issue", "Actions"]}
            rows={upstreams.map((upstream) => [
              upstream.name,
              upstream.base_url,
              upstream.priority,
              <Badge key="health" tone={upstream.last_health_status === "healthy" ? "good" : upstream.last_health_status === "unknown" ? "neutral" : "bad"}>{upstream.last_health_status}</Badge>,
              formatDate(upstream.last_health_checked_at),
              latestErrorSample(upstream.recent_error_samples),
              <div key="actions" className="flex flex-wrap gap-2">
                <Button type="button" size="icon" onClick={() => edit(upstream)} title="Edit"><Save size={15} /></Button>
                <Button type="button" size="icon" onClick={() => actionMutation.mutate({ id: upstream.id, action: "health" })} disabled={actionMutation.isPending} title="Health check"><Activity size={15} /></Button>
                <Button type="button" size="icon" onClick={() => actionMutation.mutate({ id: upstream.id, action: "disable" })} disabled={actionMutation.isPending || !upstream.enabled} title="Disable"><X size={15} /></Button>
              </div>
            ])}
          />
        )}
      </QueryState>
    </PageFrame>
  );
}

function upstreamDefaults(): UpstreamDraft {
  return {
    name: "",
    base_url: "",
    api_key: "",
    enabled: true,
    priority: "100",
    weight: "1",
    timeout_mode: "default" as const,
    timeout_ms: "",
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
    max_retries: Number(draft.max_retries),
    health_check_path: draft.health_check_path
  };
  if (draft.timeout_mode === "explicit") {
    body.timeout_ms = { mode: "explicit", value: Number(draft.timeout_ms) };
  } else if (editing) {
    body.timeout_ms = { mode: "default" };
  }
  if (!editing || draft.api_key.trim()) {
    body.api_key = draft.api_key;
  }
  return body;
}
