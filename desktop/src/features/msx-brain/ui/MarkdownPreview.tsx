import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

export function stripFrontmatter(content: string): string {
  return content.startsWith("---") ? content.split("---").slice(2).join("---").trim() : content;
}

/** Token [nao:path] → hyperlink "📁 tên-file" mở panel Não tại file đó. */
export function renderNaoTokens(text: string): string {
  return text.replace(/\[nao:([^\]]+)\]/g, (_m, rel: string) => {
    const name = rel.split("/").pop() ?? rel;
    return `[📁 ${name}](msx-brain://open?file=${encodeURIComponent(rel)})`;
  });
}

/** Kiểu chữ "cho con người đọc" — thước đo dài, giãn dòng thoải mái, phân cấp rõ. */
export function MarkdownPreview({ content }: { content: string }) {
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
                  <button
                    type="button"
                    className="msx-brain-link inline-flex items-center gap-1 rounded-md bg-amber-500/10 px-1.5 py-0.5 text-sm font-medium text-amber-700 hover:bg-amber-500/20 dark:text-amber-400"
                    onClick={() => {
                      window.dispatchEvent(
                        new CustomEvent("msx-brain-open-file", { detail: { rel } }),
                      );
                    }}
                  >
                    📎 {children}
                  </button>
                );
              }
              return <a href={href}>{children}</a>;
            },
          }}
        >
          {renderNaoTokens(stripFrontmatter(content))}
        </ReactMarkdown>
      </div>
    </div>
  );
}
