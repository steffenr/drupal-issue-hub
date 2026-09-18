import DOMPurify from "dompurify";
import { marked } from "marked";

marked.setOptions({ gfm: true, breaks: true });

/**
 * GitLab descriptions (especially issues imported from drupal.org) are mixed
 * documents: rendered HTML headings/paragraphs plus raw markdown such as
 * task lists. CommonMark HTML blocks run until a blank line, so a `- [ ]`
 * list that directly follows `</h3>` is swallowed into the verbatim HTML
 * block and rendered as plain text. Inserting a blank line after block-level
 * closing tags terminates those HTML blocks so the markdown after them is
 * parsed normally. Rendering inside the raw HTML blocks is unaffected.
 */
function terminateHtmlBlocks(body: string): string {
  return body.replace(
    /(<\/(?:h[1-6]|p|div|ol|ul|li|table|pre|blockquote)>)[ \t]*\n(?!\n)/gi,
    "$1\n\n",
  );
}

const GITLAB_HOST = "https://git.drupalcode.org";

/**
 * GitLab wraps imported content in an alert block:
 *
 *     >>> [!note] Migrated issue
 *     ...content...
 *     >>>
 *
 * CommonMark reads `>>>` as a three-level blockquote, so both the `[!note]`
 * idiom and the closing fence printed as literal text - 173 of the 629 cached
 * bodies carry this, which is what "markdown is not rendered correctly"
 * looks like. The fences go; the label stays as a bold lead-in, and the single
 * line `> [!note]` form keeps its blockquote.
 */
const ALERT_KINDS = "note|tip|important|warning|caution";

function flattenGitlabAlerts(source: string): string {
  const label = (kind: string) => kind.charAt(0).toUpperCase() + kind.slice(1).toLowerCase();
  const text = (kind: string, rest: string) =>
    `**${label(kind)}${rest.trim() ? `: ${rest.trim()}` : ""}**`;
  return source
    .replace(
      new RegExp(`^>>>\\s*\\[!(${ALERT_KINDS})\\]\\s*(.*)$`, "gim"),
      (_m, kind: string, rest: string) => text(kind, rest),
    )
    .replace(/^>>>\s*$/gm, "")
    .replace(
      new RegExp(`^(>+)\\s*\\[!(${ALERT_KINDS})\\]\\s*(.*)$`, "gim"),
      (_m, quote: string, kind: string, rest: string) => `${quote} ${text(kind, rest)}`,
    );
}

/**
 * GitLab serves its own uploads from a relative path - `/uploads/<hash>/x.png`.
 * In a browser that resolves against the site; in this app it resolves against
 * the webview itself, so the image simply never loads. Point every relative
 * upload path at the host it came from before anything is rendered.
 *
 * Relative link/image targets are a wider trap than uploads: a markdown link
 * `[patch](/diffs/12345)` in a GitLab body 404s against the webview's origin
 * in exactly the same way. Anything starting with "/" that is not a
 * protocol-relative URL is rewritten against git.drupalcode.org - GitLab's
 * own site resolves those the same way when you read the issue in a browser.
 *
 * The `drupal.orgfiles/` repair is the same class of problem: one migrated body
 * carries that mangled host, which cannot resolve anywhere.
 */
function absolutiseUploads(source: string): string {
  return source
    .replace(/drupal\.orgfiles\//gi, "drupal.org/files/")
    // markdown links: ](/path) ]('/path') and ](/path "title")
    .replace(/\]\(\s*(["']?)(\/[^")\s]+)\1(\s*"[^"]*")?\)/g, (_m, q: string, path: string, title: string) =>
      `](${q}${GITLAB_HOST}${path}${q}${title ?? ""})`)
    // HTML img src= /link href= with a relative path
    .replace(/(<img[^>]+\ssrc=)["']?(\/[^")'\s>]+)(["'])?/gi, (_m, lead: string, path: string, quote: string) =>
      `${lead}${quote}${GITLAB_HOST}${path}${quote ?? ""}`)
    .replace(/(<a[^>]+\shref=)["']?(\/\w[^")'\s>]*)(["'])?/gi, (_m, lead: string, path: string, quote: string) =>
      `${lead}${quote}${GITLAB_HOST}${path}${quote ?? ""}`);
}

/**
 * GitLab's markdown sizes an image with a trailing attribute block:
 * `![alt](url){width=900 height=409}`. marked does not know that dialect, so it
 * renders the image and then prints `{width=900 height=409}` as visible text -
 * which is what "markdown is not rendered correctly" looks like. Turn the block
 * into the attributes it means, as raw HTML, which marked passes through and
 * DOMPurify keeps (only inline colour is stripped from styles).
 */
function applyImageDimensions(source: string): string {
  return source.replace(
    /(!\[[^\]]*\]\()([^)\s]+)\)\s*\{([^}]*)\}/g,
    (match, lead: string, url: string, attrs: string) => {
      const num = (key: string) => attrs.match(new RegExp(`${key}\\s*=\\s*(\\d+)`, "i"))?.[1];
      const pct = (key: string) => attrs.match(new RegExp(`${key}\\s*=\\s*(\\d+)%`, "i"))?.[1];
      const width = pct("width") ? `${pct("width")}%` : num("width") ? `${num("width")}px` : null;
      const height = pct("height") ? `${pct("height")}%` : num("height") ? `${num("height")}px` : null;
      if (!width && !height) return match;
      const alt = (lead.match(/!\[([^\]]*)\]/) || [, ""])[1].replace(/"/g, "&quot;");
      const style = [width && `width:${width}`, height && `height:${height}`]
        .filter(Boolean)
        .join(";");
      return `<img src="${url}" alt="${alt}" style="${style}">`;
    },
  );
}

/**
 * Same class of problem, different shape: a URL that is *relative to the
 * GitLab site* (`/diffs/…`, `/-/work_items/…`) 404s against the webview's
 * origin. Absolute-ize markdown link targets that start with a single slash,
 * but leave uploads/assets alone - those are already handled by
 * absolutiseUploads, and re-writing them here would double the host.
 */
function absolutiseRelativeLinks(source: string): string {
  return source.replace(
    /\]\(\s*\/((?!uploads\/|assets\/)[^)\s]+)\)/g,
    (_m, rest: string) => `](${GITLAB_HOST}/${rest})`,
  );
}

/**
 * Everything else - remote images, external links, data URIs - is left to
 * load from where it came from. The webview fetches cross-origin media
 * directly; there is no proxy layer to configure. What this app is
 * responsible for is only that the URLs in the cached bodies point at real
 * absolute locations, because a relative one would resolve against the
 * app's own origin and 404.
 */

// drupal.org code blocks arrive as nested <span style="color: #0000BB"> - the
// site's light-theme geSHi palette. DOMPurify passes style attributes through,
// so navy on this dark background rendered at 1.49:1 and the wrapper's black
// at 1.2:1: the pasted patch was invisible exactly when it mattered. The
// colour declarations are dropped here; .rendered-body keeps themed colours
// of its own. Anchored on ";"/start so background-color and border-color pass.
//
// `img` gets the same treatment *after* the rewrite has attached its
// size-as-style: a `<img style="width:900px;height:409px">` must keep those
// two declarations or the image renders at natural size and overflows the
// pane.
DOMPurify.addHook("uponSanitizeAttribute", (node, data) => {
  if (data.attrName === "style" && node.tagName === "IMG") {
    data.attrValue = data.attrValue
      .replace(/[^;]+/gi, (decl) => /^(width|height|object-fit|max-width)$/i.test(decl.split(":")[0].trim()) ? decl : "")
      .replace(/;;+/g, ";")
      .trim();
    return;
  }
  if (data.attrName === "style") {
    data.attrValue = data.attrValue
      .replace(/(?:^|;)\s*color\s*:[^;]*/gi, "")
      .replace(/^\s*[\s;]+|[\s;]+\s*$/g, "");
  }
});

/**
 * The element allowlist GitLab applies to issue and comment markup: the
 * html-pipeline v2.12.3 SanitizationFilter::WHITELIST elements (the version
 * pinned in the git.drupalcode.org stack), plus Banzai's two additions
 * (`section` for footnotes, `input` for task-list checkboxes).
 *
 * GitLab's sanitizer (the `sanitize` gem) *unwraps* elements outside this
 * list: the tag is stripped, its children stay visible. Only `script`
 * (the WHITELIST's `remove_contents`) is dropped with its content. We must
 * reproduce that semantics *before* DOMPurify parses the string, because the
 * browser's HTML parser already did the damage: a raw, unclosed
 * `<template shadowrootmode="open">` in a description (written by an author
 * who meant it as literal code) swallows every following node into the
 * template's inert `.content` fragment — DOMPurify keeps the element and
 * then discards that fragment, so the description visibly ends where the
 * tag begins. (The real-world case: ui_patterns_library_plus !11.)
 */
const GITLAB_ALLOWED_ELEMENTS = new Set(
  (
    "h1 h2 h3 h4 h5 h6 h7 h8 br b i strong em a pre code img tt div ins del sup sub"
    + " p ol ul table thead tbody tfoot blockquote dl dt dd kbd q samp var hr ruby rt rp"
    + " li tr td th s strike summary details caption figure figcaption abbr bdo cite dfn"
    + " mark small span time wbr section input"
  ).split(" "),
);

/**
 * Mirror GitLab's sanitizer on a marked output string: unwrap every element
 * that is not on the whitelist (children kept), and drop `script`/`style`
 * with their content. Runs on already-parsed HTML, so `<template>` children
 * — which live in `el.content`, invisible to `childNodes` — are moved out
 * instead of being thrown away with the tag. The fixpoint loop catches
 * templates nested inside templates. DOMPurify still runs afterwards and
 * remains the security boundary; this pass only restores the structure GitLab
 * itself would show.
 */
function mirrorGitlabSanitizer(html: string): string {
  const doc = new DOMParser().parseFromString(html, "text/html");
  let changed = true;
  while (changed) {
    changed = false;
    for (const el of [...doc.body.querySelectorAll("*")]) {
      const tag = el.tagName.toLowerCase();
      if (GITLAB_ALLOWED_ELEMENTS.has(tag)) continue;
      changed = true;
      if (tag === "script" || tag === "style") {
        el.remove();
        continue;
      }
      const kids =
        tag === "template"
          ? [...(el as HTMLTemplateElement).content.childNodes]
          : [...el.childNodes];
      el.replaceWith(...kids);
    }
  }
  return doc.body.innerHTML;
}

/** Render untrusted issue/comment markup safely: GitLab content is markdown
 * (often mixed with rendered HTML, and in GitLab's own dialect), drupal.org
 * content is filtered HTML. Both pass through DOMPurify. */
export function renderMarkup(source: string, body: string): string {
  const prepared = applyImageDimensions(
    absolutiseRelativeLinks(absolutiseUploads(flattenGitlabAlerts(body))),
  );
  const raw =
    source === "gitlab"
      ? mirrorGitlabSanitizer(marked.parse(terminateHtmlBlocks(prepared), { async: false }) as string)
      : prepared;
  return DOMPurify.sanitize(raw);
}
