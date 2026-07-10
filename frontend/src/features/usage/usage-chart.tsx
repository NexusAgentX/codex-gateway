import { Area, AreaChart, CartesianGrid, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import type { DailyUsage } from "../../types/api";

export function UsageChart({ rows }: { rows: DailyUsage[] }) {
  const data = Object.values(
    rows.reduce<Record<string, { date: string; requests: number; errors: number; tokens: number }>>((acc, row) => {
      const bucket = acc[row.date] ?? { date: row.date, requests: 0, errors: 0, tokens: 0 };
      bucket.requests += row.request_count;
      bucket.errors += row.error_count;
      bucket.tokens += row.total_tokens;
      acc[row.date] = bucket;
      return acc;
    }, {})
  ).sort((a, b) => a.date.localeCompare(b.date));

  if (data.length === 0) {
    return null;
  }

  return (
    <section className="h-64 min-w-0 rounded-lg border border-zinc-200 bg-white p-3">
      <div className="mb-2 flex items-center justify-between gap-3">
        <h2 className="text-sm font-semibold text-zinc-950">Usage trend</h2>
        <span className="text-xs text-zinc-500">{data.length} days</span>
      </div>
      <ResponsiveContainer width="100%" height="88%">
        <AreaChart data={data} margin={{ left: 0, right: 8, top: 6, bottom: 0 }}>
          <CartesianGrid stroke="#e4e4e7" strokeDasharray="3 3" vertical={false} />
          <XAxis dataKey="date" tick={{ fontSize: 11, fill: "#71717a" }} tickLine={false} axisLine={false} minTickGap={24} />
          <YAxis tick={{ fontSize: 11, fill: "#71717a" }} tickLine={false} axisLine={false} width={42} />
          <Tooltip />
          <Area type="monotone" dataKey="requests" stroke="#047857" fill="#d1fae5" strokeWidth={2} name="Requests" />
          <Area type="monotone" dataKey="errors" stroke="#b91c1c" fill="#fee2e2" strokeWidth={2} name="Errors" />
        </AreaChart>
      </ResponsiveContainer>
    </section>
  );
}
