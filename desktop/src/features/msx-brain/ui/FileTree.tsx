import { ChevronDown, ChevronRight, File, Folder } from "lucide-react";
import { Fragment, useState } from "react";
import type { BrainEntry } from "../types";

type TreeProps = {
  entry: BrainEntry;
  depth: number;
  selectedPath: string | null;
  onOpen: (e: BrainEntry) => void;
  expanded: Set<string>;
  onToggle: (relPath: string) => void;
};

function TreeRow({ entry, depth, selectedPath, onOpen, expanded, onToggle }: TreeProps) {
  const isMd = entry.name.toLowerCase().endsWith(".md");
  const isOpen = expanded.has(entry.rel_path);
  const active = selectedPath === entry.rel_path;
  return (
    <button
      type="button"
      onClick={() => (entry.is_dir ? onToggle(entry.rel_path) : onOpen(entry))}
      className={`flex w-full min-w-0 max-w-full items-center gap-1.5 overflow-hidden rounded-md px-2 py-1.5 text-left text-sm transition-colors hover:bg-accent ${
        active ? "bg-accent font-medium" : ""
      }`}
      style={{ paddingLeft: `${8 + depth * 14}px`, width: "100%", maxWidth: "100%", boxSizing: "border-box" }}
      title={entry.name}
    >
      {entry.is_dir ? (
        isOpen ? (
          <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground" />
        ) : (
          <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" />
        )
      ) : null}
      {entry.is_dir ? (
        <Folder className="h-4 w-4 shrink-0 text-amber-500" />
      ) : (
        <File className={`h-4 w-4 shrink-0 ${isMd ? "text-sky-500" : "text-muted-foreground"}`} />
      )}
      <span className="min-w-0 flex-1 truncate">{entry.name}</span>
    </button>
  );
}

export function FileTree({
  entries,
  selectedPath,
  onOpen,
  naoChon,
}: {
  entries: BrainEntry[];
  selectedPath: string | null;
  onOpen: (e: BrainEntry) => void;
  naoChon?: string | null;
}) {
  // lọc theo não con đang chọn: chỉ hiện "Nao Bo Phan/<naoChon>/..." + mọi thứ ngoài Nao Bo Phan
  const visible = naoChon
    ? entries.filter(
        (e) =>
          !e.rel_path.startsWith("Nao Bo Phan/") ||
          e.rel_path.startsWith(`Nao Bo Phan/${naoChon}/`) ||
          e.rel_path === "Nao Bo Phan" ||
          e.rel_path === `Nao Bo Phan/${naoChon}`,
      )
    : entries;
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  const byParent = new Map<string, BrainEntry[]>();
  for (const e of visible) {
    const parent = e.rel_path.includes("/")
      ? e.rel_path.slice(0, e.rel_path.lastIndexOf("/"))
      : "";
    if (!byParent.has(parent)) byParent.set(parent, []);
    byParent.get(parent)!.push(e);
  }

  function toggle(relPath: string) {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(relPath)) next.delete(relPath);
      else next.add(relPath);
      return next;
    });
  }

  function renderLevel(parent: string, depth: number): React.ReactNode[] {
    return (byParent.get(parent) ?? []).map((e) => (
      <Fragment key={e.rel_path}>
        <TreeRow
          entry={e}
          depth={depth}
          selectedPath={selectedPath}
          onOpen={onOpen}
          expanded={expanded}
          onToggle={toggle}
        />
        {e.is_dir && expanded.has(e.rel_path) ? renderLevel(e.rel_path, depth + 1) : null}
      </Fragment>
    ));
  }

  return (
    <div className="flex h-full w-full flex-col overflow-y-auto overflow-x-hidden p-2">
      {renderLevel("", 0)}
      {(byParent.get("") ?? []).length === 0 && (
        <div className="p-4 text-sm text-muted-foreground">Không có tài liệu nào bạn được xem.</div>
      )}
    </div>
  );
}
