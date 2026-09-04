import { ChevronDown, ChevronRight, File, Folder, Search } from "lucide-react";
import { Fragment, useMemo, useState } from "react";
import type { BrainEntry } from "../types";
import { filterByNao, type NaoDef } from "../lib/naoDefs";

type TreeProps = {
  entry: BrainEntry;
  depth: number;
  selectedPath: string | null;
  onOpen: (e: BrainEntry) => void;
  expanded: Set<string>;
  onToggle: (relPath: string) => void;
};

function parentLabel(rel: string): string {
  const i = rel.lastIndexOf("/");
  if (i < 0) return "";
  const parent = rel.slice(0, i);
  const j = parent.lastIndexOf("/");
  return j < 0 ? parent : parent.slice(j + 1);
}

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
  allPaths,
}: {
  entries: BrainEntry[];
  selectedPath: string | null;
  onOpen: (e: BrainEntry) => void;
  naoChon: NaoDef | null;
  allPaths: string[];
}) {
  const [query, setQuery] = useState("");
  const visible = filterByNao(entries, naoChon, allPaths);
  const q = query.trim().toLowerCase();
  const searched = q
    ? visible.filter((e) => !e.is_dir && e.name.toLowerCase().includes(q)).slice(0, 40)
    : null;
  const recent = useMemo(() => {
    if (q) return [];
    return visible
      .filter((e) => !e.is_dir && (e.mtime ?? 0) > 0)
      .sort((a, b) => (b.mtime ?? 0) - (a.mtime ?? 0))
      .slice(0, 5);
  }, [visible, q]);
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
    <div className="flex h-full w-full flex-col overflow-hidden">
      <div className="border-b p-2">
        <div className="relative">
          <Search className="absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
          <input
            value={query}
            onChange={(ev) => setQuery(ev.target.value)}
            placeholder="Tìm trong não…"
            className="w-full rounded-md border bg-transparent py-1.5 pl-8 pr-2 text-sm outline-none focus:ring-1 focus:ring-ring"
          />
        </div>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto overflow-x-hidden p-2">
      {searched ? (
        searched.length ? (
          searched.map((e) => (
            <button
              key={e.rel_path}
              type="button"
              onClick={() => onOpen(e)}
              className="flex w-full min-w-0 items-center gap-1.5 truncate rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent"
              title={e.rel_path}
            >
              <File className="h-4 w-4 shrink-0 text-sky-500" />
              <span className="min-w-0 flex-1 truncate">{e.name}</span>
              <span className="shrink-0 truncate text-[11px] text-muted-foreground">{parentLabel(e.rel_path)}</span>
            </button>
          ))
        ) : (
          <div className="p-4 text-sm text-muted-foreground">Không thấy "{query}"</div>
        )
      ) : (
        <>
          {recent.length > 0 && (
            <div className="mb-1">
              <div className="px-2 py-1 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
                🕘 Mới cập nhật
              </div>
              {recent.map((e) => (
                <button
                  key={e.rel_path}
                  type="button"
                  onClick={() => onOpen(e)}
                  className={`flex w-full min-w-0 items-center gap-1.5 truncate rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent ${
                    selectedPath === e.rel_path ? "bg-accent font-medium" : ""
                  }`}
                  title={e.rel_path}
                >
                  <File className="h-4 w-4 shrink-0 text-emerald-500" />
                  <span className="min-w-0 flex-1 truncate">{e.name}</span>
                </button>
              ))}
            </div>
          )}
          {renderLevel("", 0)}
          {!q && (byParent.get("") ?? []).length === 0 && (
            <div className="p-4 text-sm text-muted-foreground">Không có tài liệu nào bạn được xem.</div>
          )}
        </>
      )}
      </div>
    </div>
  );
}
