import { useQuery } from "@tanstack/react-query";
import { ListChecks, X } from "lucide-react";
import { useState } from "react";
import { PageFrame } from "../components/layout/page-frame";
import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/form";
import { QueryState } from "../components/ui/query-state";
import { DataTable } from "../components/ui/table";
import { apiFetch } from "../lib/api/client";
import { formatDate, formatNumber, requestFilterQuery, statusTone } from "../lib/format";
import { isAdmin, useSession } from "../lib/auth/session";
import type { Model, RequestLog, Upstream } from "../types/api";

export function RequestsPage() {
  const { session } = useSession();
  const [filters, setFilters] = useState({ user_id: "", key_id: "", model_id: "", upstream_id: "", status: "", from: "", to: "" });
  if (!session) return null;
  const admin = isAdmin(session);
  const query = useQuery({
    queryKey: ["requests", session.token, admin, filters],
    queryFn: async () => {
      const requestPath = `${admin ? "/api/admin/requests" : "/api/requests"}${requestFilterQuery(filters)}`;
      const [requests, upstreams, models] = await Promise.all([
        apiFetch<RequestLog[]>(requestPath, { token: session.token }),
        admin ? apiFetch<Upstream[]>("/api/admin/upstreams", { token: session.token }) : Promise.resolve([]),
        admin ? apiFetch<Model[]>("/api/admin/models", { token: session.token }) : Promise.resolve([])
      ]);
      return { requests, upstreams, models };
    }
  });

  return (
    <PageFrame title="Requests" icon={ListChecks} onRefresh={() => void query.refetch()} refreshing={query.isFetching}>
      <div className="grid min-w-0 grid-cols-[repeat(7,minmax(110px,1fr))_auto] items-end gap-2 max-[980px]:grid-cols-2 max-[760px]:grid-cols-1">
        {admin ? <Input name="filter_user_id" value={filters.user_id} onChange={(event) => setFilters({ ...filters, user_id: event.target.value })} placeholder="User ID" /> : null}
        <Input name="filter_key_id" value={filters.key_id} onChange={(event) => setFilters({ ...filters, key_id: event.target.value })} placeholder="Key ID" />
        <Input name="filter_model_id" value={filters.model_id} onChange={(event) => setFilters({ ...filters, model_id: event.target.value })} placeholder="Model ID" />
        <Input name="filter_upstream_id" value={filters.upstream_id} onChange={(event) => setFilters({ ...filters, upstream_id: event.target.value })} placeholder="Upstream ID" />
        <Input name="filter_status" value={filters.status} onChange={(event) => setFilters({ ...filters, status: event.target.value })} placeholder="Status" inputMode="numeric" />
        <Input name="filter_from" value={filters.from} onChange={(event) => setFilters({ ...filters, from: event.target.value })} type="date" aria-label="From" />
        <Input name="filter_to" value={filters.to} onChange={(event) => setFilters({ ...filters, to: event.target.value })} type="date" aria-label="To" />
        <Button type="button" onClick={() => setFilters({ user_id: "", key_id: "", model_id: "", upstream_id: "", status: "", from: "", to: "" })}>
          <X size={16} />
          Clear
        </Button>
      </div>
      <QueryState query={query}>
        {({ requests, upstreams, models }) => {
          const upstreamNames = new Map(upstreams.map((upstream) => [upstream.id, upstream.name]));
          const modelNames = new Map(models.map((model) => [model.id, model.public_name]));
          return (
            <DataTable
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
      </QueryState>
    </PageFrame>
  );
}
