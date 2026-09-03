import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

export function stripFrontmatter(content: string): string {
  return content.startsWith("---") ? content.split("---").slice(2).join("---").trim() : content;
}

/** Token [nao:path] và wikilink [[Tên]] → chip bấm được. */
export function renderBrainLinks(text: string): string {
  const wiki: string[] = [];
  let t = text.replace(/\[\[([^\]]+)\]\]/g, (_m, name: string) => {
    wiki.push(name);
    return `[WIKILINK${wiki.length - 1}]`;
  });
  t = t.replace(/\[nao:([^\]]+)\]/g, (_m, rel: string) => `[NAO${encodeURIComponent(rel)}]`);
  // khôi phục wikilink thành markdown link riêng (tránh trùng cú pháp)
  t = t.replace(/\[WIKILINK(\d+)\]/g, (_m, i: string) => {
    const name = wiki[Number(i)];
    return `[${name}](msx-brain://wiki?name=${encodeURIComponent(name)})`;
  });
  t = t.replace(/\[NAO([^\]]*)\]/g, (_m, enc: string) => {
    const rel = decodeURIComponent(enc);
    const name = rel.split("/").pop() ?? rel;
    return `[${name}](msx-brain://open?file=${encodeURIComponent(rel)})`;
  });
  return t;
}

export function BrainLinkChip({ children, onClick, title }: { children: React.ReactNode; onClick: () => void; title?: string }) {
  return (
    <button
      type="button"
      className="msx-brain-link inline-flex max-w-full items-center gap-1 rounded-lg border border-amber-500/40 bg-amber-500/10 px-2 py-0.5 align-baseline text-sm font-medium text-amber-700 transition-colors hover:bg-amber-500/20 dark:text-amber-400"
      title={title}
      onClick={(ev) => {
        ev.stopPropagation();
        onClick();
      }}
    >
      📎 {children}
    </button>
  );
}

/** Kiểu chữ "cho con người đọc" — dùng .msx-md trong globals.css. */
export function MarkdownPreview({
  content,
  onOpenFile,
  onOpenWiki,
}: {
  content: string;
  onOpenFile?: (rel: string) => void;
  onOpenWiki?: (name: string) => void;
  kind?: string;
}) {
  return (
    <div className="msx-md h-full overflow-y-auto px-8 py-6">
      <div className="mx-auto max-w-3xl">
        <ReactMarkdown
          remarkPlugins={[remarkGfm]}
          components={{
            a: ({ href, children }) => {
              if (href?.startsWith("msx-brain://open?file=")) {
                const rel = decodeURIComponent(href.replace("msx-brain://open?file=", ""));
                return (
                  <BrainLinkChip
                    title={rel}
                    onClick={() => onOpenFile?.(rel)}
                  >
                    {children}
                  </BrainLinkChip>
                );
              }
              if (href?.startsWith("msx-brain://wiki?name=")) {
                const name = decodeURIComponent(href.replace("msx-brain://wiki?name=", ""));
                return (
                  <BrainLinkChip
                    title={`[[${name}]]`}
                    onClick={() => onOpenWiki?.(name)}
                  >
                    {children}
                  </BrainLinkChip>
                );
              }
              return <a href={href}>{children}</a>;
            },
          }}
        >
          {renderBrainLinks(stripFrontmatter(content))}
        </ReactMarkdown>
      </div>
    </div>
  );
}
