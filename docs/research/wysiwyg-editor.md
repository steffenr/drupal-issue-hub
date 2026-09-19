# WYSIWYG Editor for Issues & Comments — Research + Prototype

**Branch:** `research/wysiwyg-editor` (off `develop`)
**Prototype:** `/tmp/wysiwyg-prototypes` (Vite + React; one route per editor, `/?editor=<name>`)
**Verdict up front:** keep the current markdown-into-textarea model for *comments*; if a
WYSIWYG editor is wanted for *issue bodies*, **TipTap is the best fit**. Details below.

## Why this is a hard fit for this app

The whole rendering/sanitization pipeline in this app is built around the fact that
**what gets written is not what we control**:

- GitLab issue bodies and comments are markdown (mixed with d.o-imported HTML — see
  `terminateHtmlBlocks()` in `render.ts`).
- drupal.org bodies are filtered HTML.
- Everything is stored as the source text and rendered through
  `renderMarkup(source, body)` → marked → DOMPurify.

So a WYSIWYG editor is only useful if it can **emit markdown** (not just HTML) that
round-trips cleanly with what GitLab/d.o already accept. All four candidates were
evaluated against that constraint.

## Candidates tested

| Editor | Model | Markdown round-trip | Bundle (min/gzip, this app's deps) | Notes |
|---|---|---|---|---|
| **TipTap** (+`tiptap-markdown`) | WYSIWYG, ProseMirror | **Yes** — `editor.storage.markdown.getMarkdown()` | 1810 / **454** KB | Best API. React-first, tiny hand-rolled toolbar. |
| **Lexical** (+`@lexical/markdown`) | WYSIWYG, React-native | **Yes** — `$convertToMarkdownString` | 1460 / **344** KB | Fastest; markdown I/O cleanest; but node/plugin API, more glue. |
| **Milkdown Crepe** | WYSIWYG, markdown-native | Partial — markdown kept internally; reading it out is ctx/plugin work | ~ (crepe is heavy) | Batteries-included, own toolbar + dark theme; least React-idiomatic. |
| **@uiw/react-md-editor** | **Split view** (textarea + live preview) | Trivial (the source *is* markdown) | 2521 / **659** KB | Not WYSIWYG; lightest to *integrate*, heaviest bundle (CodeMirror). |
| **marked only** (baseline) | textarea + `marked` preview | N/A | **57 / 14 KB** | What the app does today. |

> Bundle figures are per-editor Vite builds of the isolated prototype entry with
> minification. Real cost in-app is lower (shared React) and the marked baseline is
> already a dependency.

## Per-editor findings

### TipTap — recommended
- True WYSIWYG: headings/bold/lists/tasks/quotes/code-blocks render inline as you type.
- `tiptap-markdown` gives **markdown in and out** with a one-line accessor. Caveat:
  the accessor is `editor.storage.markdown.getMarkdown()` (a storage hook), not a
  top-level method — easy to wrap.
- TipTap is framework-agnostic but has first-class React bindings; our toolbar is just
  React buttons calling `editor.chain()`.
- Proven in Tauri + React + TipTap desktop markdown editors (multiple OSS apps), so
  no WKWebView surprises.
- Cost: ~454 KB gzip on top of the existing bundle. Acceptable for a desktop app.

### Lexical
- Fastest engine of the big three (Meta-backed, React-native).
- `@lexical/markdown` round-trips markdown both ways cleanly (`TRANSFORMERS`).
- Downside: the API is node/plugin-based, less React-idiomatic than TipTap; more
  "glue" to wire up the same feature set. No decisive advantage here over TipTap.

### Milkdown Crepe
- Markdown-native WYSIWYG; ships its own toolbar, menus and a dark theme.
- Heaviest and least React-idiomatic (ctx/plugin-based); pulling its chrome out to
  match our app's surface is more work than it saves.

### @uiw/react-md-editor
- **Not** WYSIWYG — it's a textarea + live rendered preview (GitHub-flavored).
- Simplest possible mental model (the source *is* the markdown), but the split view
  isn't "WYSIWYG", and its bundle is the *largest* of the group (CodeMirror inside).
- Viable as a "bump the existing textarea" middle ground, not as a WYSIWYG answer.

## Recommendation

1. **Comments:** keep the current `textarea` (markdown, ⌘/Ctrl+Enter). Comments are
   short; a full WYSIWYG editor is overkill and the token-gated GitLab write path is
   already fine. This matches the "simple" intent.
2. **Issue bodies:** adopt **TipTap** with `tiptap-markdown`, hand-rolled React
   toolbar, and wire `editor.storage.markdown.getMarkdown()` into the existing
   `update_issue`/`create_issue` commands. This slots into the app's
   `renderMarkup` sanitization pipeline because the *saved* artifact stays markdown.
3. **Do not** adopt Milkdown or react-md-editor for this use case.

## What landed on this branch (TipTap, with a plain-markdown escape hatch)

- `src/components/TipTapEditor.tsx` — the editor:
  - Markdown in **and** out: seeded from the stored body, and `onMarkdown` fires
    on every keystroke with `editor.storage.markdown.getMarkdown()`, so **save
    posts the same markdown artifact** the plain editor and `renderMarkup`
    already understand. No HTML/Markdown divergence.
  - Toolbar: H1/H2, B/I/S, inline code, bullet/numbered/task lists, quote,
    code block, link.
  - Two prototyping gotchas, now handled:
    - **Drop `@tiptap/extension-link`** — `tiptap-markdown` bundles its own
      `link` mark; adding both throws "Duplicate extension names: link". Links
      still round-trip via the built-in mark; the 🔗 button *removes* links
      (no `window.prompt` — hard rule 0).
    - `getMarkdown` lives on `editor.storage.markdown`, not the editor root.
- `src/components/IssueDetail.tsx` — in the "Edit issue" form, the body renders
  the TipTap editor **only** for a writable GitLab issue; drupal.org rows and
  token-less GitLab rows keep the plain textarea. An "Edit as plain markdown /
  Switch to WYSIWYG" toggle lets anything that breaks be fixed in the textarea;
  both modes save through the same `api.updateIssue`.
- `src/App.css` — theme-aware styles for the toolbar + document.

Bundle cost: ~454 KB gzip for the TipTap subtree (fine for desktop).

## Prototype

Run (already installed at `/tmp/wysiwyg-prototypes`):

```sh
cd /tmp/wysiwyg-prototypes
npx vite --port 5199
# then open http://localhost:5199/?editor=tiptap   (or lexical / milkdown /
# reactmdeditor / marked-only)
```

Each route renders the four real candidates plus the `marked` baseline, seeds them
with a sample Drupal-issue-style body, and shows the **resulting markdown** live in a
box below the editor so you can see what would actually be saved. The bundle sizes
above were produced by `scripts/build-*.mjs` (isolated per-editor Vite builds).

## Try the integrated editor in the app

```sh
npm run tauri dev
# open a GitLab work item → "Edit issue" → the body is now WYSIWYG
# ("Edit as plain markdown" under the editor is the escape hatch)
```

## Open questions

- Drupal issue bodies lean heavily on `Problem/Motivation`, `Steps to reproduce`,
  `Proposed resolution` headings — a small "insert section heading" shortcut set
  would be the next highest-value add to the toolbar.
- Bundle budget: +454 KB gzip (TipTap) is fine for a desktop app but noted.
