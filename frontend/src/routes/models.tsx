import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Boxes, Plus } from "lucide-react";
import { useState, type FormEvent } from "react";
import { PageFrame } from "../components/layout/page-frame";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/form";
import { Notice } from "../components/ui/notice";
import { QueryState } from "../components/ui/query-state";
import { EmptyState } from "../components/ui/state";
import { DataTable } from "../components/ui/table";
import { ModelEditor } from "../features/models/model-editor";
import { apiFetch } from "../lib/api/client";
import { messageForError, yesNo } from "../lib/format";
import { useSession } from "../lib/auth/session";
import type { Model, ModelMapping, Upstream } from "../types/api";

export function ModelsPage() {
  const { session } = useSession();
  const queryClient = useQueryClient();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [createName, setCreateName] = useState("");
  const [message, setMessage] = useState<string | null>(null);
  if (!session) return null;
  const queryKey = ["models", session.token];
  const query = useQuery({
    queryKey,
    queryFn: async () => {
      const [models, upstreams] = await Promise.all([
        apiFetch<Model[]>("/api/admin/models", { token: session.token }),
        apiFetch<Upstream[]>("/api/admin/upstreams", { token: session.token })
      ]);
      const mappingPairs = await Promise.all(
        models.map(async (model) => [model.id, await apiFetch<ModelMapping[]>(`/api/admin/models/${model.id}/mappings`, { token: session.token })] as const)
      );
      return { models, upstreams, mappings: Object.fromEntries(mappingPairs) as Record<string, ModelMapping[]> };
    }
  });
  const invalidate = () => queryClient.invalidateQueries({ queryKey });
  const createMutation = useMutation({
    mutationFn: () =>
      apiFetch<Model>("/api/admin/models", {
        method: "POST",
        token: session.token,
        body: { public_name: createName, description: null, enabled: true, visible_to_users: true }
      }),
    onSuccess() {
      setCreateName("");
      void invalidate();
    },
    onError(error) {
      setMessage(messageForError(error));
    }
  });

  function createModel(event: FormEvent) {
    event.preventDefault();
    setMessage(null);
    createMutation.mutate();
  }

  return (
    <PageFrame title="Models" icon={Boxes} onRefresh={() => void query.refetch()} refreshing={query.isFetching}>
      <form className="grid min-w-0 grid-cols-[repeat(4,minmax(140px,1fr))_auto] items-end gap-2 max-[980px]:grid-cols-2 max-[760px]:grid-cols-1" onSubmit={createModel}>
        <Input name="model_public_name" value={createName} onChange={(event) => setCreateName(event.target.value)} placeholder="Public model name" required />
        <Button type="submit" variant="primary" disabled={createMutation.isPending}><Plus size={16} />Create</Button>
      </form>
      {message ? <Notice tone={message.includes("(") ? "error" : "note"}>{message}</Notice> : null}
      <QueryState query={query}>
        {({ models, upstreams, mappings }) => {
          const selected = models.find((model) => model.id === selectedId) ?? models[0] ?? null;
          return models.length === 0 ? (
            <EmptyState text="No models configured." />
          ) : (
            <div className="grid min-w-0 grid-cols-[minmax(0,0.9fr)_minmax(340px,1fr)] items-start gap-4 max-[980px]:grid-cols-1">
              <DataTable
                empty="No models configured."
                columns={["Model", "Visible", "Enabled", "Mappings"]}
                rows={models.map((model) => [
                  <Button key="model" type="button" variant="link" onClick={() => setSelectedId(model.id)}>{model.public_name}</Button>,
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
                  onChanged={() => void invalidate()}
                  onMessage={setMessage}
                />
              ) : null}
            </div>
          );
        }}
      </QueryState>
    </PageFrame>
  );
}
