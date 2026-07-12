import type { DailyUsage } from "../types/api";
import { ApiClientError } from "./api/client";

export function formatDate(value: string | null | undefined) {
  if (!value) return "-";
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

export function formatNumber(value: number) {
  return new Intl.NumberFormat().format(value);
}

export function formatPercent(value: number) {
  return `${new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 }).format(value * 100)}%`;
}

export function statusTone(status: number | null): "good" | "bad" | "neutral" {
  if (!status) return "neutral";
  return status >= 400 ? "bad" : "good";
}

export function summarizeUsage(rows: DailyUsage[]) {
  return rows.reduce(
    (totals, row) => ({
      requests: totals.requests + row.request_count,
      errors: totals.errors + row.error_count,
      tokens: totals.tokens + row.total_tokens,
      latency: totals.latency + row.latency_ms_sum
    }),
    { requests: 0, errors: 0, tokens: 0, latency: 0 }
  );
}

export function latestErrorSample(value: string | null | undefined) {
  if (!value) return "-";
  try {
    const samples = JSON.parse(value) as Array<{ at?: string; status?: string; error?: string }>;
    const latest = samples.at(-1);
    return latest ? [latest.error, latest.status, formatDate(latest.at)].filter(Boolean).join(" / ") : "-";
  } catch {
    return value;
  }
}

export function yesNo(value: boolean | number) {
  return value ? "yes" : "no";
}

export type RequestFilters = {
  user_id: string;
  key_id: string;
  model_id: string;
  upstream_id: string;
  status: string;
  from: string;
  to: string;
  from_exact: string;
  to_exact: string;
  latency_min_ms: string;
  latency_max_ms: string;
};

export function emptyRequestFilters(): RequestFilters {
  return { user_id: "", key_id: "", model_id: "", upstream_id: "", status: "", from: "", to: "", from_exact: "", to_exact: "", latency_min_ms: "", latency_max_ms: "" };
}

export function requestFiltersFromSearch(params: URLSearchParams): RequestFilters {
  const rawFrom = params.get("from") ?? "";
  const rawTo = params.get("to") ?? "";
  const from = normalizeDateInput(rawFrom);
  const to = normalizeDateInput(rawTo);
  return {
    ...emptyRequestFilters(),
    user_id: params.get("user_id") ?? "",
    key_id: params.get("key_id") ?? params.get("api_key_id") ?? "",
    model_id: params.get("model_id") ?? "",
    upstream_id: params.get("upstream_id") ?? "",
    status: params.get("status") ?? "",
    from,
    to,
    from_exact: rawFrom && rawFrom !== from ? rawFrom : "",
    to_exact: rawTo && rawTo !== to ? rawTo : "",
    latency_min_ms: params.get("latency_min_ms") ?? "",
    latency_max_ms: params.get("latency_max_ms") ?? ""
  };
}

export function normalizeDateInput(value: string | null | undefined) {
  const trimmed = value?.trim();
  if (!trimmed) return "";
  const dateOnly = trimmed.match(/^(\d{4}-\d{2}-\d{2})$/);
  if (dateOnly) return dateOnly[1];
  const timestampDate = trimmed.match(/^(\d{4}-\d{2}-\d{2})[T ]/);
  if (timestampDate) return timestampDate[1];
  const parsed = new Date(trimmed);
  return Number.isNaN(parsed.getTime()) ? "" : parsed.toISOString().slice(0, 10);
}

export function requestFilterQuery(filters: RequestFilters) {
  const params = new URLSearchParams();
  const entries = {
    user_id: filters.user_id,
    key_id: filters.key_id,
    model_id: filters.model_id,
    upstream_id: filters.upstream_id,
    status: filters.status,
    from: filters.from_exact || filters.from,
    to: filters.to_exact || filters.to,
    latency_min_ms: filters.latency_min_ms,
    latency_max_ms: filters.latency_max_ms
  };
  for (const [key, value] of Object.entries(entries)) {
    const trimmed = value.trim();
    if (trimmed) {
      params.set(key, trimmed);
    }
  }
  const query = params.toString();
  return query ? `?${query}` : "";
}

export function requestDrilldownPath(filters: Partial<{ user_id: string | null; key_id: string | null; model_id: string | null; upstream_id: string | null; status: string | number | null; from: string | null; to: string | null; latency_min_ms: string | number | null; latency_max_ms: string | number | null }>) {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(filters)) {
    if (value === null || value === undefined) continue;
    const trimmed = String(value).trim();
    if (trimmed) {
      params.set(key, trimmed);
    }
  }
  const query = params.toString();
  return `/requests${query ? `?${query}` : ""}`;
}

export function fieldName(label: string) {
  return label.toLowerCase().replace(/[^a-z0-9]+/g, "_").replace(/^_|_$/g, "");
}

export function messageForError(error: unknown) {
  if (error instanceof ApiClientError) {
    return `${error.message} (${error.code})`;
  }
  if (error instanceof Error) {
    return error.message;
  }
  return "Request failed";
}
