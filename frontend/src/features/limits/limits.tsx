import { Save } from "lucide-react";
import { useEffect, useState, type FormEvent } from "react";
import { Button } from "../../components/ui/button";
import { Field, Select } from "../../components/ui/form";
import { Notice } from "../../components/ui/notice";
import { NumberInput } from "../../components/ui/number-input";
import { Stat } from "../../components/ui/stat";
import { formatNumber, messageForError } from "../../lib/format";
import type { LimitPolicy, LimitSubjectState } from "../../types/api";

type LimitMode = "inherit" | "limited" | "unlimited";
export type LimitPatchPayload = {
  mode: LimitMode;
  value?: number;
};

export function LimitSummary({ state }: { state: LimitSubjectState | undefined }) {
  if (!state) return null;
  return (
    <div className="grid min-w-0 grid-cols-4 gap-3 max-[980px]:grid-cols-2 max-[760px]:grid-cols-1">
      <Stat label="Request quota" value={limitCell(state.request_quota)} />
      <Stat label="Token budget" value={limitCell(state.token_budget)} />
      <Stat label="Rate limit" value={limitCell(state.rate_limit)} />
      <Stat
        label="Concurrency"
        value={state.concurrency.limit === null ? `${state.concurrency.in_flight} live / unlimited` : `${state.concurrency.remaining} left / ${state.concurrency.limit}`}
      />
    </div>
  );
}

export function LimitPolicyEditor({
  title,
  policy,
  compact,
  onSave
}: {
  title: string;
  policy: LimitPolicy;
  compact?: boolean;
  onSave: (body: Record<string, number | LimitPatchPayload>) => Promise<void>;
}) {
  const [draft, setDraft] = useState(() => policyDraft(policy));
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    setDraft(policyDraft(policy));
  }, [policy]);

  async function submit(event: FormEvent) {
    event.preventDefault();
    setMessage(null);
    const validationError = validateLimitDraft(draft);
    if (validationError) {
      setMessage(validationError);
      return;
    }
    setBusy(true);
    try {
      await onSave(limitPolicyBody(draft, policy.scope));
      setMessage("Limits saved.");
    } catch (err) {
      setMessage(messageForError(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form
      className={`grid min-w-0 items-end gap-2 rounded-lg border border-zinc-200 bg-white p-3 ${compact ? "grid-cols-[repeat(4,minmax(120px,1fr))_auto]" : "grid-cols-[repeat(4,minmax(130px,1fr))_auto]"} max-[980px]:grid-cols-2 max-[760px]:grid-cols-1`}
      onSubmit={submit}
    >
      <h2 className="col-span-full text-base font-semibold text-zinc-950">{title}</h2>
      <LimitModeInput label="Request quota" mode={draft.request_quota_mode} value={draft.request_quota} allowInherit={policy.scope !== "system"} onMode={(value) => setDraft({ ...draft, request_quota_mode: value })} onValue={(value) => setDraft({ ...draft, request_quota: value })} onInvalidValue={setMessage} />
      <NumberInput label="Request window seconds" value={draft.request_window_seconds} onChange={(value) => setDraft({ ...draft, request_window_seconds: value })} />
      <LimitModeInput label="Token budget" mode={draft.token_quota_mode} value={draft.token_quota} allowInherit={policy.scope !== "system"} onMode={(value) => setDraft({ ...draft, token_quota_mode: value })} onValue={(value) => setDraft({ ...draft, token_quota: value })} onInvalidValue={setMessage} />
      <NumberInput label="Token window seconds" value={draft.token_window_seconds} onChange={(value) => setDraft({ ...draft, token_window_seconds: value })} />
      <LimitModeInput label="Rate requests" mode={draft.rate_limit_mode} value={draft.rate_limit_requests} allowInherit={policy.scope !== "system"} onMode={(value) => setDraft({ ...draft, rate_limit_mode: value })} onValue={(value) => setDraft({ ...draft, rate_limit_requests: value })} onInvalidValue={setMessage} />
      <NumberInput label="Rate window seconds" value={draft.rate_limit_window_seconds} onChange={(value) => setDraft({ ...draft, rate_limit_window_seconds: value })} />
      <LimitModeInput label="Concurrency" mode={draft.concurrency_mode} value={draft.concurrency_limit} allowInherit={policy.scope !== "system"} onMode={(value) => setDraft({ ...draft, concurrency_mode: value })} onValue={(value) => setDraft({ ...draft, concurrency_limit: value })} onInvalidValue={setMessage} />
      <Button type="submit" variant="primary" disabled={busy}>
        <Save size={16} />
        Save limits
      </Button>
      {message ? <Notice tone={message === "Limits saved." ? "note" : "error"}>{message}</Notice> : null}
    </form>
  );
}

export function limitCell(bucket: { limit: number | null; used: number; remaining: number | null }) {
  if (bucket.limit === null) {
    return `${formatNumber(bucket.used)} used / unlimited`;
  }
  return `${formatNumber(bucket.remaining ?? 0)} left / ${formatNumber(bucket.limit)}`;
}

function LimitModeInput({
  label,
  mode,
  value,
  allowInherit,
  onMode,
  onValue,
  onInvalidValue
}: {
  label: string;
  mode: LimitMode;
  value: string;
  allowInherit: boolean;
  onMode: (value: LimitMode) => void;
  onValue: (value: string) => void;
  onInvalidValue: (message: string) => void;
}) {
  const name = label.toLowerCase().replace(/[^a-z0-9]+/g, "_").replace(/^_|_$/g, "");
  return (
    <Field label={label}>
      <div className="grid grid-cols-[minmax(0,1fr)_minmax(82px,0.8fr)] gap-1.5">
        <Select name={`${name}_mode`} value={mode} onChange={(event) => onMode(event.target.value as LimitMode)}>
          {allowInherit ? <option value="inherit">inherit</option> : null}
          <option value="limited">limited</option>
          <option value="unlimited">unlimited</option>
        </Select>
        <NumberInput
          label={`${label} value`}
          name={`${name}_value`}
          value={value}
          onChange={onValue}
          disabled={mode !== "limited"}
          required={mode === "limited"}
          onInvalid={() => onInvalidValue(`${label} must have a non-negative numeric value when mode is limited.`)}
        />
      </div>
    </Field>
  );
}

function policyDraft(policy: LimitPolicy) {
  return {
    request_quota: valueOrBlank(policy.request_quota),
    request_quota_mode: modeOrUnlimited(policy.request_quota_mode),
    request_window_seconds: String(policy.request_window_seconds),
    token_quota: valueOrBlank(policy.token_quota),
    token_quota_mode: modeOrUnlimited(policy.token_quota_mode),
    token_window_seconds: String(policy.token_window_seconds),
    rate_limit_requests: valueOrBlank(policy.rate_limit_requests),
    rate_limit_mode: modeOrUnlimited(policy.rate_limit_mode),
    rate_limit_window_seconds: String(policy.rate_limit_window_seconds),
    concurrency_limit: valueOrBlank(policy.concurrency_limit),
    concurrency_mode: modeOrUnlimited(policy.concurrency_mode)
  };
}

function valueOrBlank(value: number | null) {
  return value === null ? "" : String(value);
}

function modeOrUnlimited(value: string): LimitMode {
  return value === "inherit" || value === "limited" || value === "unlimited" ? value : "unlimited";
}

function limitPolicyBody(draft: ReturnType<typeof policyDraft>, scope: string): Record<string, number | LimitPatchPayload> {
  const body: Record<string, number | LimitPatchPayload> = {
    request_quota: limitPatch(draft.request_quota_mode, draft.request_quota, "Request quota"),
    token_quota: limitPatch(draft.token_quota_mode, draft.token_quota, "Token budget"),
    rate_limit_requests: limitPatch(draft.rate_limit_mode, draft.rate_limit_requests, "Rate requests"),
    concurrency_limit: limitPatch(draft.concurrency_mode, draft.concurrency_limit, "Concurrency")
  };
  if (scope === "system" || draft.request_quota_mode === "limited") {
    body.request_window_seconds = Number(draft.request_window_seconds || 86400);
  }
  if (scope === "system" || draft.token_quota_mode === "limited") {
    body.token_window_seconds = Number(draft.token_window_seconds || 86400);
  }
  if (scope === "system" || draft.rate_limit_mode === "limited") {
    body.rate_limit_window_seconds = Number(draft.rate_limit_window_seconds || 60);
  }
  return body;
}

function validateLimitDraft(draft: ReturnType<typeof policyDraft>) {
  return (
    validateLimitedValue("Request quota", draft.request_quota_mode, draft.request_quota) ??
    validateLimitedValue("Token budget", draft.token_quota_mode, draft.token_quota) ??
    validateLimitedValue("Rate requests", draft.rate_limit_mode, draft.rate_limit_requests) ??
    validateLimitedValue("Concurrency", draft.concurrency_mode, draft.concurrency_limit)
  );
}

function validateLimitedValue(label: string, mode: LimitMode, value: string) {
  if (mode !== "limited") return null;
  if (parseLimitValue(value) === null) {
    return `${label} must have a non-negative numeric value when mode is limited.`;
  }
  return null;
}

function limitPatch(mode: LimitMode, value: string, label: string): LimitPatchPayload {
  if (mode !== "limited") return { mode };
  const parsed = parseLimitValue(value);
  if (parsed === null) {
    throw new Error(`${label} must have a non-negative numeric value when mode is limited.`);
  }
  return { mode, value: parsed };
}

function parseLimitValue(value: string) {
  if (value.trim() === "") return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed >= 0 ? parsed : null;
}
