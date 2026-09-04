import { Node, mergeAttributes } from "@tiptap/core";

export const NAO_FILE_NODE_NAME = "naoFile";

export interface NaoFileAttrs {
  relPath: string;
  name: string;
}

/** Inline atom: chip "📎 Tên-file" gọn trong composer; gửi đi thành markdown link. */
export const NaoFileNode = Node.create({
  name: NAO_FILE_NODE_NAME,
  group: "inline",
  inline: true,
  atom: true,
  selectable: true,
  draggable: true,

  addAttributes() {
    return {
      relPath: { default: "" },
      name: { default: "" },
    };
  },

  parseHTML() {
    return [{ tag: `span[data-${NAO_FILE_NODE_NAME}]` }];
  },

  renderHTML({ HTMLAttributes }) {
    const attrs = HTMLAttributes as NaoFileAttrs;
    return [
      "span",
      mergeAttributes(HTMLAttributes, {
        [`data-${NAO_FILE_NODE_NAME}`]: "",
        "data-rel-path": attrs.relPath,
        "data-name": attrs.name,
        class: "msx-nao-chip",
      }),
      `📎 ${attrs.name}`,
    ];
  },

  renderText({ node }) {
    const attrs = node.attrs as NaoFileAttrs;
    return `[${attrs.name}](msx-brain://open?file=${encodeURIComponent(attrs.relPath)})`;
  },

  addStorage() {
    return {
      markdown: {
        serialize(state: { write: (s: string) => void }, node: { attrs: NaoFileAttrs }) {
          const { relPath, name } = node.attrs;
          state.write(`[${name}](msx-brain://open?file=${encodeURIComponent(relPath)})`);
        },
      },
    };
  },
});
