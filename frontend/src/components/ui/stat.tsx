export function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0 rounded-lg border border-zinc-200 bg-white p-3">
      <span className="block text-xs font-semibold uppercase text-zinc-500">{label}</span>
      <strong className="mt-2 block truncate text-xl font-semibold text-zinc-950">{value}</strong>
    </div>
  );
}
