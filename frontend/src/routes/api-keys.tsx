import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, Copy, KeyRound, Plus, Trash2, X } from "lucide-react";
import { useState, type FormEvent } from "react";
import { PageFrame } from "../components/layout/page-frame";
import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import { Input, Select } from "../components/ui/form";
import { Notice } from "../components/ui/notice";
import { QueryState } from "../components/ui/query-state";
import { DataTable } from "../components/ui/table";
import { LimitPolicyEditor, limitCell } from "../features/limits/limits";
import { apiFetch } from "../lib/api/client";
import { messageForError } from "../lib/format";
import { isAdmin, useSession } from "../lib/auth/session";
import type { AdminLimitState, ApiKeySummary, User, UserLimitState } from "../types/api";

export function ApiKeysPage() {
  const { session } = useSession();
  const queryClient = useQueryClient();
  const [name, setName] = useState("");
  const [expiresAt, setExpiresAt] = useState("");
  const [userId, setUserId] = useState(session?.user.id ?? "");
  const [plaintext, setPlaintext] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  if (!session) return null;
  const admin = isAdmin(session);
  const queryKey = ["api-keys", session.token, admin];
  const query = useQuery({
    queryKey,
    queryFn: async () => {
      const [keys, users, limits] = await Promise.all([
        apiFetch<ApiKeySummary[]>(admin ? "/api/admin/api-keys" : "/api/api-keys", { token: session.token }),
        admin ? apiFetch<User[]>("/api/admin/users", { token: session.token }) : Promise.resolve([]),
        admin ? apiFetch<AdminLimitState>("/api/admin/limits", { token: session.token }) : apiFetch<UserLimitState>("/api/limits", { token: session.token })
      ]);
      return { keys, users, limits };
    }
  });
  const invalidate = () => queryClient.invalidateQueries({ queryKey });
  const createMutation = useMutation({
    mutationFn: async () => {
      const expires_at = expiresAt ? new Date(expiresAt).toISOString() : null;
      const body = admin ? { user_id: userId, name, expires_at } : { name, expires_at };
      return apiFetch<{ key: ApiKeySummary; plaintext: string }>(admin ? "/api/admin/api-keys" : "/api/api-keys", {
        method: "POST",
        token: session.token,
        body
      });
    },
    onSuccess(created) {
      setPlaintext(created.plaintext);
      setName("");
      setExpiresAt("");
      void invalidate();
    },
    onError(error) {
      setActionError(messageForError(error));
    }
  });
  const statusMutation = useMutation({
    mutationFn: ({ id, status }: { id: string; status: "disable" | "revoke" }) =>
      apiFetch(`${admin ? "/api/admin" : "/api"}/api-keys/${id}/${status}`, {
        method: "POST",
        token: session.token
      }),
    onSuccess() {
      setActionError(null);
      void invalidate();
    },
    onError(error) {
      setActionError(messageForError(error));
    }
  });

  function createKey(event: FormEvent) {
    event.preventDefault();
    setActionError(null);
    setPlaintext(null);
    createMutation.mutate();
  }

  return (
    <PageFrame title="API Keys" icon={KeyRound} onRefresh={() => void query.refetch()} refreshing={query.isFetching}>
      <form className="grid min-w-0 grid-cols-[repeat(4,minmax(140px,1fr))_auto] items-end gap-2 max-[980px]:grid-cols-2 max-[760px]:grid-cols-1" onSubmit={createKey}>
        {admin ? (
          <Select name="user_id" value={userId} onChange={(event) => setUserId(event.target.value)} aria-label="User">
            {query.data?.users.map((user) => (
              <option key={user.id} value={user.id}>{user.email}</option>
            ))}
          </Select>
        ) : null}
        <Input name="key_name" value={name} onChange={(event) => setName(event.target.value)} placeholder="Key name" required />
        <Input name="expires_at" value={expiresAt} onChange={(event) => setExpiresAt(event.target.value)} type="datetime-local" aria-label="Expires at" />
        <Button type="submit" variant="primary" disabled={createMutation.isPending}>
          <Plus size={16} />
          Create
        </Button>
      </form>
      {plaintext ? <OneTimeSecret value={plaintext} /> : null}
      {actionError ? <Notice tone="error">{actionError}</Notice> : null}
      <QueryState query={query}>
        {({ keys, users, limits }) => (
          <>
            <DataTable
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
                  <div key="actions" className="flex flex-wrap gap-2">
                    <Button type="button" size="icon" onClick={() => statusMutation.mutate({ id: key.id, status: "disable" })} disabled={statusMutation.isPending || key.status !== "active"} title="Disable">
                      <X size={15} />
                    </Button>
                    <Button type="button" size="icon" onClick={() => statusMutation.mutate({ id: key.id, status: "revoke" })} disabled={statusMutation.isPending || key.status === "revoked"} title="Revoke">
                      <Trash2 size={15} />
                    </Button>
                  </div>
                ];
              })}
            />
            {admin ? (
              <div className="grid gap-2">
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
                        await invalidate();
                      }}
                    />
                  ) : null;
                })}
              </div>
            ) : null}
          </>
        )}
      </QueryState>
    </PageFrame>
  );
}

function OneTimeSecret({ value }: { value: string }) {
  const [copied, setCopied] = useState(false);
  async function copy() {
    await navigator.clipboard.writeText(value);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1200);
  }
  return (
    <div className="grid min-w-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-3 rounded-lg border border-zinc-200 bg-white p-3 max-[760px]:grid-cols-1">
      <div className="grid min-w-0 gap-1">
        <strong className="text-sm text-zinc-950">Plaintext key</strong>
        <code className="overflow-auto rounded-md bg-zinc-100 p-2 text-sm text-emerald-950">{value}</code>
      </div>
      <Button type="button" onClick={copy}>
        {copied ? <Check size={16} /> : <Copy size={16} />}
        {copied ? "Copied" : "Copy"}
      </Button>
    </div>
  );
}
