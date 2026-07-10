import type { ReactNode } from "react";
import { EmptyState } from "./state";

export function DataTable({ columns, rows, empty }: { columns: string[]; rows: ReactNode[][]; empty: string }) {
  if (rows.length === 0) {
    return <EmptyState text={empty} />;
  }
  return (
    <div className="table-shell">
      <table className="data-table">
        <thead>
          <tr>{columns.map((column) => <th key={column}>{column}</th>)}</tr>
        </thead>
        <tbody>
          {rows.map((row, rowIndex) => (
            <tr key={rowIndex}>
              {row.map((cell, cellIndex) => (
                <td key={cellIndex} data-label={columns[cellIndex]}>
                  {cell}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
