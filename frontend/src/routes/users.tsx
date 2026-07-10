import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Plus, Save, Users } from "lucide-react";
import { useState, type FormEvent } from "react";
import { PageFrame } from "../components/layout/page-frame";
import { Button } from "../components/ui/button";
import { Input, Select } from "../components/ui/form";
import { Notice } from "../components/ui/notice";
import { QueryState } from "../components/ui/query-state";
import { EmptyState } from "../components/ui/state";
import { LimitPolicyEditor } from "../features/limits/limits";
import { apiFetch } from "../lib/api/client";
import { formatDate, messageForError } from "../lib/format";
import { useSession, type Session } from "../lib/auth/session";
import type { AdminLimitState, LimitSubjectState, User } from "../types/api";

export function UsersPage() {
  const { session } = useSession();
  const queryClient = useQueryClient();
  const [create, setCreate] = useState({ email: "", password: "", role: "user", display_name: "" });
  const [message, setMessage] = useState<string | null>(null);
  if (!session) return null;
  const queryKey = ["users", session.token];
  const query = useQuery({
    queryKey,
    queryFn: async () => {
      const [users, limits] = await Promise.all([
        apiFetch<User[]>("/api/admin/users", { token: session.token }),
        apiFetch<AdminLimitState>("/api/admin/limits", { token: session.token })
      ]);
      return { users, limits };
    }
  });
  const invalidate = () => queryClient.invalidateQueries({ queryKey });
  const createMutation = useMutation({
    mutationFn: () =>
      apiFetch("/api/admin/users", {
        method: "POST",
        token: session.token,
        body: { ...create, display_name: create.display_name || null }
      }),
    onSuccess() {
      setCreate({ email: "", password: "", role: "user", display_name: "" });
      void invalidate();
    },
    onError(error) {
      setMessage(messageForError(error));
    }
  });

  function createUser(event: FormEvent) {
    event.preventDefault();
    setMessage(null);
    createMutation.mutate();
  }

  return (
    <PageFrame title="Users" icon={Users} onRefresh={() => void query.refetch()} refreshing={query.isFetching}>
      <form className="grid min-w-0 grid-cols-[repeat(4,minmax(140px,1fr))_auto] items-end gap-2 max-[980px]:grid-cols-2 max-[760px]:grid-cols-1" onSubmit={createUser}>
        <Input name="user_email" value={create.email} onChange={(event) => setCreate({ ...create, email: event.target.value })} placeholder="Email" type="email" required />
        <Input name="user_password" value={create.password} onChange={(event) => setCreate({ ...create, password: event.target.value })} placeholder="Password" type="password" required />
        <Select name="user_role" value={create.role} onChange={(event) => setCreate({ ...create, role: event.target.value })}>
          <option value="user">user</option>
          <option value="admin">admin</option>
        </Select>
        <Input name="user_display_name" value={create.display_name} onChange={(event) => setCreate({ ...create, display_name: event.target.value })} placeholder="Display name" />
        <Button type="submit" variant="primary" disabled={createMutation.isPending}><Plus size={16} />Create</Button>
      </form>
      {message ? <Notice tone={message.includes("(") ? "error" : "note"}>{message}</Notice> : null}
      <QueryState query={query}>
        {({ users, limits }) => (
          <div className="grid min-w-0 gap-2">
            {users.length === 0 ? <EmptyState text="No users found." /> : users.map((user) => (
              <UserRow
                key={user.id}
                session={session}
                user={user}
                limits={limits.users.find((limit) => limit.subject_id === user.id)}
                onChanged={() => void invalidate()}
                onMessage={setMessage}
              />
            ))}
          </div>
        )}
      </QueryState>
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
    <section className="grid min-w-0 grid-cols-[minmax(190px,1.2fr)_minmax(140px,1fr)_110px_120px_auto_minmax(140px,1fr)_auto] items-center gap-2 rounded-lg border border-zinc-200 bg-white p-3 max-[980px]:grid-cols-2 max-[760px]:grid-cols-1">
      <div className="grid min-w-0 gap-1">
        <strong className="truncate text-sm text-zinc-950">{user.email}</strong>
        <span className="truncate text-xs text-zinc-500">{formatDate(user.last_login_at)}</span>
      </div>
      <Input name="user_row_display_name" value={draft.display_name} onChange={(event) => setDraft({ ...draft, display_name: event.target.value })} placeholder="Display name" />
      <Select name="user_row_role" value={draft.role} onChange={(event) => setDraft({ ...draft, role: event.target.value })}>
        <option value="user">user</option>
        <option value="admin">admin</option>
      </Select>
      <Select name="user_row_status" value={draft.status} onChange={(event) => setDraft({ ...draft, status: event.target.value })}>
        <option value="active">active</option>
        <option value="disabled">disabled</option>
      </Select>
      <Button type="button" onClick={save}><Save size={15} />Save</Button>
      <Input name="user_row_new_password" value={draft.password} onChange={(event) => setDraft({ ...draft, password: event.target.value })} placeholder="New password" type="password" />
      <Button type="button" onClick={resetPassword} disabled={draft.password.length < 8}>Reset</Button>
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
