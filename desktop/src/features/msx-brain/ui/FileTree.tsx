import { File, Folder, FolderOpen } from "lucide-react";
import { useState } from "react";
import type { BrainEntry } from "../types";

function TreeRow({
  entry,
  depth,
  selectedPath,
  onOpen,
}: {
  entry: BrainEntry;
  depth: number;
  selectedPath: string | null;
  onOpen: (e: BrainEntry) => void;
}) {
  const [expanded, setExpanded] = useState(depth < 1);
  const isMd = entry.name.toLowerCase().endsWith(".md");
  const active = selectedPath === entry.rel_path;
  return (
    <>
      <button
        type="button"
        onClick={() => {
          if (entry.is_dir) setExpanded((v) => !v);
          else onOpen(entry);
        }}
        className={`flex w-full items-center gap-1.5 rounded-md px-2 py-1.5 text-left text-sm transition-colors hover:bg-accent ${
          active ? "bg-accent font-medium" : ""
        }`}
        style={{ paddingLeft: `${8 + depth * 14}px` }}
        title={entry.name}
      >
        {entry.is_dir ? (
          expanded ? (
            <FolderOpen className="h-4 w-4 shrink-0 text-amber-500" />
          ) : (
            <Folder className="h-4 w-4 shrink-0 text-amber-500" />
          )
        ) : (
          <File className={`h-4 w-4 shrink-0 ${isMd ? "text-sky-500" : "text-muted-foreground"}`} />
        )}
        <span className="truncate">{entry.name}</span>
      </button>
    </>
  );
}

export function FileTree({
  entries,
  selectedPath,
  onOpen,
}: {
  entries: BrainEntry[];
  selectedPath: string | null;
  onOpen: (e: BrainEntry) => void;
}) {
  const byParent = new Map<string, BrainEntry[]>();
  for (const e of entries) {
    const parent = e.rel_path.includes("/") ? e.rel_path.slice(0, e.rel_path.lastIndexOf("/")) : "";
    if (!byParent.has(parent)) byParent.set(parent, []);
    byParent.get(parent)!.push(e);
  }
  const roots = byParent.get("") ?? [];

  function renderLevel(parent: string, depth: number): React.ReactNode {
    return (byParent.get(parent) ?? []).map((e) => (
      <TreeRowWithChildren key={e.rel_path} entry={e} depth={depth} />
    ));
  }

  function TreeRowWithChildren({
    entry,
    depth,
  }: {
    entry: BrainEntry;
    depth: number;
  }) {
    return (
      <>
        <TreeRow entry={entry} depth={depth} selectedPath={selectedPath} onOpen={onOpen} />
        {entry.is_dir && byParent.has(entry.rel_path)
          ? renderLevel(entry.rel_path, depth + 1)
          : null}
      </>
    );
  }

  return (
    <div className="flex h-full flex-col overflow-y-auto p-2">
      {renderLevel("", 0)}
      {roots.length === 0 && (
        <div className="p-4 text-sm text-muted-foreground">Không có tài liệu nào bạn được xem.</div>
      )}
    </div>
  );
}
