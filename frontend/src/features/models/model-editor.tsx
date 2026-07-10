import { Plus, Save, X } from "lucide-react";
import { useState, type FormEvent } from "react";
import { Button } from "../../components/ui/button";
import { CheckboxLine, Select, Textarea, Input } from "../../components/ui/form";
import { NumberInput } from "../../components/ui/number-input";
import { EmptyState } from "../../components/ui/state";
import { apiFetch } from "../../lib/api/client";
import { messageForError } from "../../lib/format";
import type { Session } from "../../lib/auth/session";
import type { Model, ModelMapping, Upstream } from "../../types/api";

export function ModelEditor({
  session,
  model,
  upstreams,
  mappings,
  onChanged,
  onMessage
}: {
  session: Session;
  model: Model;
  upstreams: Upstream[];
  mappings: ModelMapping[];
  onChanged: () => void;
  onMessage: (message: string | null) => void;
}) {
  const [description, setDescription] = useState(model.description ?? "");
  const [enabled, setEnabled] = useState(Boolean(model.enabled));
  const [visible, setVisible] = useState(Boolean(model.visible_to_users));
  const [draft, setDraft] = useState({ upstream_id: upstreams[0]?.id ?? "", upstream_model_name: "", priority: "100", weight: "1" });

  async function saveModel(event: FormEvent) {
    event.preventDefault();
    onMessage(null);
    try {
      await apiFetch(`/api/admin/models/${model.id}`, {
        method: "PATCH",
        token: session.token,
        body: { description, enabled, visible_to_users: visible }
      });
      onChanged();
    } catch (err) {
      onMessage(messageForError(err));
    }
  }

  async function addMapping(event: FormEvent) {
    event.preventDefault();
    onMessage(null);
    try {
      await apiFetch(`/api/admin/models/${model.id}/mappings`, {
        method: "POST",
        token: session.token,
        body: {
          upstream_id: draft.upstream_id,
          upstream_model_name: draft.upstream_model_name,
          enabled: true,
          priority: Number(draft.priority),
          weight: Number(draft.weight)
        }
      });
      setDraft({ ...draft, upstream_model_name: "" });
      onChanged();
    } catch (err) {
      onMessage(messageForError(err));
    }
  }

  return (
    <section className="grid min-w-0 gap-3 rounded-lg border border-zinc-200 bg-white p-3">
      <form className="grid gap-3" onSubmit={saveModel}>
        <h2 className="text-base font-semibold text-zinc-950">{model.public_name}</h2>
        <Textarea name="model_description" value={description} onChange={(event) => setDescription(event.target.value)} placeholder="Description" />
        <div className="flex flex-wrap gap-3">
          <CheckboxLine><input name="model_enabled" className="h-4 w-4" type="checkbox" checked={enabled} onChange={(event) => setEnabled(event.target.checked)} />Enabled</CheckboxLine>
          <CheckboxLine><input name="model_visible" className="h-4 w-4" type="checkbox" checked={visible} onChange={(event) => setVisible(event.target.checked)} />Visible</CheckboxLine>
        </div>
        <Button type="submit" variant="primary"><Save size={16} />Save model</Button>
      </form>
      <form className="grid min-w-0 grid-cols-[repeat(4,minmax(120px,1fr))_auto] items-end gap-2 max-[980px]:grid-cols-2 max-[760px]:grid-cols-1" onSubmit={addMapping}>
        <Select name="mapping_upstream_id" value={draft.upstream_id} onChange={(event) => setDraft({ ...draft, upstream_id: event.target.value })} required>
          {upstreams.map((upstream) => <option key={upstream.id} value={upstream.id}>{upstream.name}</option>)}
        </Select>
        <Input name="mapping_upstream_model_name" value={draft.upstream_model_name} onChange={(event) => setDraft({ ...draft, upstream_model_name: event.target.value })} placeholder="Upstream model" required />
        <NumberInput label="Priority" value={draft.priority} onChange={(value) => setDraft({ ...draft, priority: value })} />
        <NumberInput label="Weight" value={draft.weight} onChange={(value) => setDraft({ ...draft, weight: value })} />
        <Button type="submit"><Plus size={16} />Add mapping</Button>
      </form>
      <div className="grid gap-2">
        {mappings.length === 0 ? <EmptyState text="No mappings for this model." /> : mappings.map((mapping) => (
          <MappingRow key={mapping.id} session={session} mapping={mapping} upstreams={upstreams} onChanged={onChanged} onMessage={onMessage} />
        ))}
      </div>
    </section>
  );
}

function MappingRow({ session, mapping, upstreams, onChanged, onMessage }: { session: Session; mapping: ModelMapping; upstreams: Upstream[]; onChanged: () => void; onMessage: (message: string | null) => void }) {
  const [draft, setDraft] = useState({
    upstream_id: mapping.upstream_id,
    upstream_model_name: mapping.upstream_model_name,
    enabled: Boolean(mapping.enabled),
    priority: String(mapping.priority),
    weight: String(mapping.weight)
  });

  async function save() {
    onMessage(null);
    try {
      await apiFetch(`/api/admin/model-mappings/${mapping.id}`, {
        method: "PATCH",
        token: session.token,
        body: {
          upstream_id: draft.upstream_id,
          upstream_model_name: draft.upstream_model_name,
          enabled: draft.enabled,
          priority: Number(draft.priority),
          weight: Number(draft.weight)
        }
      });
      onChanged();
    } catch (err) {
      onMessage(messageForError(err));
    }
  }

  async function disable() {
    onMessage(null);
    try {
      await apiFetch(`/api/admin/model-mappings/${mapping.id}/disable`, { method: "POST", token: session.token });
      onChanged();
    } catch (err) {
      onMessage(messageForError(err));
    }
  }

  return (
    <div className="grid min-w-0 grid-cols-[minmax(140px,1fr)_minmax(150px,1fr)_95px_95px_auto_36px_36px] items-center gap-2 rounded-lg border border-zinc-200 p-2 max-[980px]:grid-cols-2 max-[760px]:grid-cols-1">
      <Select name="mapping_row_upstream_id" value={draft.upstream_id} onChange={(event) => setDraft({ ...draft, upstream_id: event.target.value })}>
        {upstreams.map((upstream) => <option key={upstream.id} value={upstream.id}>{upstream.name}</option>)}
      </Select>
      <Input name="mapping_row_upstream_model_name" value={draft.upstream_model_name} onChange={(event) => setDraft({ ...draft, upstream_model_name: event.target.value })} />
      <NumberInput label="Priority" value={draft.priority} onChange={(value) => setDraft({ ...draft, priority: value })} />
      <NumberInput label="Weight" value={draft.weight} onChange={(value) => setDraft({ ...draft, weight: value })} />
      <CheckboxLine><input name="mapping_row_enabled" className="h-4 w-4" type="checkbox" checked={draft.enabled} onChange={(event) => setDraft({ ...draft, enabled: event.target.checked })} />Enabled</CheckboxLine>
      <Button type="button" size="icon" onClick={save} title="Save"><Save size={15} /></Button>
      <Button type="button" size="icon" onClick={disable} disabled={!mapping.enabled} title="Disable"><X size={15} /></Button>
    </div>
  );
}
