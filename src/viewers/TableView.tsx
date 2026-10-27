import { asText } from "./dig";

/** Rows of flat objects, as produced by the `list.table` tool or any array of plain objects. */
export function isRows(v: unknown): v is Record<string, unknown>[] {
  return Array.isArray(v) && v.length > 0 && v.every((r) => typeof r === "object" && r !== null && !Array.isArray(r));
}

export function TableView({ rows }: { rows: Record<string, unknown>[] }) {
  const columns = Array.from(new Set(rows.flatMap((r) => Object.keys(r))));
  return (
    <div className="table-scroll">
      <table>
        <thead>
          <tr>{columns.map((c) => <th key={c}>{c}</th>)}</tr>
        </thead>
        <tbody>
          {rows.map((r, i) => (
            <tr key={i}>
              {columns.map((c) => {
                const v = r[c];
                return (
                  <td key={c} className={typeof v === "boolean" ? "mono" : ""}>
                    {typeof v === "boolean" ? (v ? "yes" : "no") : asText(v)}
                  </td>
                );
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
