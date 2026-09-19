import { useEffect, useState } from "react";
import { Editor, EditorContent } from "@tiptap/react";
import { StarterKit } from "@tiptap/starter-kit";
import { CodeBlock } from "@tiptap/extension-code-block";
import { TaskList } from "@tiptap/extension-task-list";
import { TaskItem } from "@tiptap/extension-task-item";
import { Placeholder } from "@tiptap/extension-placeholder";
import { Markdown } from "tiptap-markdown";

const btnStyle = (active: boolean) => ({
  background: "var(--bg-raised)",
  color: "var(--text)",
  border: active ? "1px solid var(--accent)" : "1px solid var(--border)",
  borderRadius: 6,
  padding: "3px 8px",
  cursor: "pointer",
  fontSize: 12,
  fontFamily: "inherit",
});

/**
 * TipTap's ProseMirror document model has no HTML element nodes — inline
 * HTML like `<link rel="stylesheet">` or block-level CSS is consumed
 * silently by markdown-it's HTML-block rule and disappears from the
 * document. Escape all angle-bracket tags before seeding so they render
 * as visible literal text instead of being swallowed.
 */
function escapeHtmlForMarkdown(src: string): string {
  // Replace < followed by any sequence of characters up to the next >
  // (i.e. a tag) with its entity form so markdown-it treats it as text.
  return src.replace(/<[^>]*>/g, (tag) => tag.replace(/</g, "&lt;").replace(/>/g, "&gt;"));
}

/**
 * The tiptap-markdown extension exposes its serializer on the storage
 * object, not on the editor root — wrap it before relying on it.
 */
function getMarkdown(ed: Editor): string {
  const get = (
    ed.storage as { markdown?: { getMarkdown?: () => string } }
  ).markdown?.getMarkdown;
  return get ? get() : ed.getText();
}

/**
 * A TipTap WYSIWYG editor for the GitLab issue body. The document is seeded
 * from the stored markdown and re-emitted as markdown on every change, so the
 * app keeps saving what the backend understands (the same artifact the
 * plain-textarea mode and `renderMarkup` both work with).
 */
export function TipTapEditor({
  value,
  onMarkdown,
  placeholder = "Start writing…",
  onCancel,        // optional: shown as a ✕ next to the toolbar (toggle/cancel UX)
}: {
  value: string;
  onMarkdown: (md: string) => void;
  placeholder?: string;
  onCancel?: () => void;
}) {
  const [editor, setEditor] = useState<Editor | null>(null);
  // Force a re-render on every editor transaction so isActive() in the
  // toolbar reflects the current selection — without this the buttons
  // only update on mount, not when the cursor moves.
  const [, setTick] = useState(0);

  useEffect(() => {
    const ed = new Editor({
      extensions: [
        StarterKit.configure({ codeBlock: false }),
        CodeBlock,
        // Links are handled by tiptap-markdown's own Link mark (adding
        // @tiptap/extension-link too produced a duplicate-name warning).
        TaskList,
        TaskItem.configure({ nested: true }),
        Placeholder.configure({ placeholder }),
        Markdown,
      ],
      content: value ? escapeHtmlForMarkdown(value) : "",
      onUpdate: ({ editor: e }) => onMarkdown(getMarkdown(e)),
    });
    onMarkdown(getMarkdown(ed));
    setEditor(ed);
    const onTick = () => setTick((t) => t + 1);
    ed.on("transaction", onTick);
    return () => {
      ed.off("transaction", onTick);
      ed.destroy();
      setEditor(null);
    };
    // The seed only matters on mount; afterwards the editor drives state.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const actions: [string, string, () => void, boolean][] = editor
    ? [
        ["H1", "Heading 1", () => editor.chain().focus().toggleHeading({ level: 1 }).run(), editor.isActive("heading", { level: 1 })],
        ["H2", "Heading 2", () => editor.chain().focus().toggleHeading({ level: 2 }).run(), editor.isActive("heading", { level: 2 })],
        ["B", "Bold", () => editor.chain().focus().toggleBold().run(), editor.isActive("bold")],
        ["I", "Italic", () => editor.chain().focus().toggleItalic().run(), editor.isActive("italic")],
        ["S", "Strikethrough", () => editor.chain().focus().toggleStrike().run(), editor.isActive("strike")],
        ["</>", "Inline code", () => editor.chain().focus().toggleCode().run(), editor.isActive("code")],
        ["•", "Bullet list", () => editor.chain().focus().toggleBulletList().run(), editor.isActive("bulletList")],
        ["1.", "Numbered list", () => editor.chain().focus().toggleOrderedList().run(), editor.isActive("orderedList")],
        ["☑", "Task list", () => editor.chain().focus().toggleTaskList().run(), editor.isActive("taskList")],
        ["❝", "Quote", () => editor.chain().focus().toggleBlockquote().run(), editor.isActive("blockquote")],
        ["{ }", "Code block", () => editor.chain().focus().toggleCodeBlock().run(), editor.isActive("codeBlock")],
        [
          "🔗",
          "Remove link from the selection",
          () => {
            // Removes the link mark without asking for a URL (window.prompt is
            // unavailable in WKWebView — hard rule 0). For a link with a real
            // URL, use the plain-markdown editor's [text](url) form.
            editor
              .chain()
              .focus()
              .extendMarkRange("link")
              .unsetLink()
              .run();
          },
          false,
        ],
      ]
    : [];

  return (
    <div className="tiptap-editor">
      {editor && (
        <div
          className="tiptap-toolbar sticky"
          role="toolbar"
          aria-label="Formatting"
        >
          {actions.map((a) => {
            const [label, title, fn, active] = a;
            return (
              <button
                key={label}
                type="button"
                title={title}
                aria-pressed={active}
                style={btnStyle(active)}
                onMouseDown={(e) => {
                  e.preventDefault();
                  fn();
                }}
              >
                {label}
              </button>
            );
          })}
          {onCancel && (
            <button
              className="tiptap-toolbar-cancel"
              type="button"
              title="Cancel editing — restores the original body"
              onClick={onCancel}
            >
              ✕
            </button>
          )}
        </div>
      )}
      <div className="tiptap-doc" style={{ minHeight: 200 }}>
        {editor && <EditorContent editor={editor} />}
      </div>
    </div>
  );
}
