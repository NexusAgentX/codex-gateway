import { Stat } from "../../components/ui/stat";
import { formatNumber, formatPercent } from "../../lib/format";
import type { UsageSummary } from "../../types/api";

export function UsageSummaryStats({ summary }: { summary: UsageSummary }) {
  const avgLatency = summary.totals.request_count
    ? `${Math.round(summary.totals.latency_ms_sum / summary.totals.request_count)} ms`
    : "-";
  return (
    <div className="grid min-w-0 grid-cols-4 gap-3 max-[980px]:grid-cols-2 max-[760px]:grid-cols-1">
      <Stat label="Requests" value={formatNumber(summary.totals.request_count)} />
      <Stat label="Tokens" value={formatNumber(summary.totals.total_tokens)} />
      <Stat label="Error rate" value={formatPercent(summary.totals.error_rate)} />
      <Stat label="Avg latency" value={avgLatency} />
    </div>
  );
}
