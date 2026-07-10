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

export function yesNo(value: number) {
  return value ? "yes" : "no";
}

export function requestFilterQuery(filters: { user_id: string; key_id: string; model_id: string; upstream_id: string; status: string; from: string; to: string }) {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(filters)) {
    const trimmed = value.trim();
    if (trimmed) {
      params.set(key, trimmed);
    }
  }
  const query = params.toString();
  return query ? `?${query}` : "";
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
