import ReactMarkdown from "react-markdown";

export function stripFrontmatter(content: string): string {
  return content.startsWith("---") ? content.split("---").slice(2).join("---").trim() : content;
}

export function MarkdownPreview({ content }: { content: string }) {
  return (
    <div className="prose prose-sm dark:prose-invert max-w-none overflow-y-auto p-4">
      <ReactMarkdown>{stripFrontmatter(content)}</ReactMarkdown>
    </div>
  );
}
