import { useBrainTree } from "../lib/useBrainTree";
import { FileTree } from "./FileTree";
import { MarkdownPreview } from "./MarkdownPreview";

export function BrainPanel({
  vaultRoot,
  myPubkey,
}: {
  vaultRoot: string;
  myPubkey: string;
}) {
  const { entries, selected, content, error, open } = useBrainTree(vaultRoot, myPubkey);
  return (
    <div className="flex h-full w-full">
      <div className="w-80 shrink-0 border-r">
        <div className="border-b px-3 py-2 text-sm font-semibold">Não MSX</div>
        <FileTree entries={entries} selectedPath={selected?.rel_path ?? null} onOpen={open} />
      </div>
      <div className="min-w-0 flex-1">
        {error && <div className="p-4 text-destructive">{error}</div>}
        {selected ? (
          <MarkdownPreview content={content} />
        ) : (
          <div className="flex h-full items-center justify-center text-muted-foreground">
            Chọn một tài liệu để xem
          </div>
        )}
      </div>
    </div>
  );
}
