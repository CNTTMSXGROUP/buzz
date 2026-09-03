import type { BrainEntry } from "../types";

export function FileTree({
  entries,
  selectedPath,
  onOpen,
}: {
  entries: BrainEntry[];
  selectedPath: string | null;
  onOpen: (e: BrainEntry) => void;
}) {
  const areas: string[] = [];
  for (const e of entries) {
    if (!areas.includes(e.area)) areas.push(e.area);
  }
  return (
    <div className="flex h-full flex-col overflow-y-auto p-2 text-sm">
      {areas.map((area) => (
        <div key={area} className="mb-2">
          <div className="px-2 py-1 text-xs font-semibold text-muted-foreground">{area}</div>
          {entries
            .filter((e) => e.area === area)
            .map((e) => (
              <button
                key={e.rel_path}
                type="button"
                onClick={() => onOpen(e)}
                className={`block w-full truncate rounded px-4 py-1 text-left hover:bg-accent ${
                  selectedPath === e.rel_path ? "bg-accent font-medium" : ""
                }`}
                style={{ paddingLeft: `${12 + (e.rel_path.split("/").length - 1) * 12}px` }}
                title={e.name}
              >
                {e.is_dir ? "📁" : "📄"} {e.name}
              </button>
            ))}
        </div>
      ))}
      {entries.length === 0 && (
        <div className="p-4 text-muted-foreground">Không có tài liệu nào bạn được xem.</div>
      )}
    </div>
  );
}
