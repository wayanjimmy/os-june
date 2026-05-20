import { IconBold } from "central-icons-filled/IconBold";
import { IconBulletList } from "central-icons-filled/IconBulletList";
import { IconH1 } from "central-icons-filled/IconH1";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";

type NotePreviewProps = {
  /** Stable id of the note — used to remount the editor when it changes. */
  noteId: string;
  markdown: string;
  onChange: (markdown: string) => void;
  emptyPlaceholder?: string;
};

/**
 * Editable note surface rendered as a quiet read-style doc (Granola/Fellow
 * flavored): headings become serif section titles, lists/paragraphs are
 * styled for scanning, markdown chrome is diminished.
 *
 * It is an *uncontrolled* `contentEditable` region — React seeds the initial
 * HTML once (keyed on `noteId`) and then stays out of the way so the caret
 * never jumps. On blur we serialize the DOM back to markdown and report up.
 *
 * The selection toolbar floats above highlighted text with a deliberately
 * slim command set (bold / italic / copy) — the same lightweight treatment
 * Fellow uses for inline comments.
 */
export function NotePreview({
  noteId,
  markdown,
  onChange,
  emptyPlaceholder,
}: NotePreviewProps) {
  const ref = useRef<HTMLDivElement>(null);
  const [toolbar, setToolbar] = useState<{ x: number; y: number } | null>(
    null,
  );

  // Built once per note — feeding this back through React on every keystroke
  // would fight the caret, so it is intentionally not reactive to `markdown`.
  const initialHtml = useMemo(() => markdownToHtml(markdown), [noteId]);

  // Empty tracking lives entirely on the DOM (no React state) so re-renders
  // can never overwrite the contentEditable's children while the user types.
  useLayoutEffect(() => {
    syncEmpty();
  });

  useEffect(() => {
    function update() {
      const selection = window.getSelection();
      const root = ref.current;
      if (!selection || selection.isCollapsed || !root) {
        setToolbar(null);
        return;
      }
      const anchor = selection.anchorNode;
      if (!anchor || !root.contains(anchor)) {
        setToolbar(null);
        return;
      }
      const rect = selection.getRangeAt(0).getBoundingClientRect();
      if (rect.width === 0 && rect.height === 0) {
        setToolbar(null);
        return;
      }
      setToolbar({ x: rect.left + rect.width / 2, y: rect.top - 8 });
    }
    document.addEventListener("selectionchange", update);
    window.addEventListener("scroll", update, true);
    return () => {
      document.removeEventListener("selectionchange", update);
      window.removeEventListener("scroll", update, true);
    };
  }, []);

  function commit() {
    const root = ref.current;
    if (root) onChange(htmlToMarkdown(root));
  }

  function syncEmpty() {
    const root = ref.current;
    if (!root) return;
    const empty = !root.textContent?.trim();
    if (empty) root.setAttribute("data-empty", "true");
    else root.removeAttribute("data-empty");
  }

  // Belt-and-suspenders fallback: regardless of how the space landed
  // (keyboard layout, IME, paste), if a block now starts with "# ", "- "
  // or "* ", convert it. Fires after the DOM has the new content.
  function handleInput() {
    syncEmpty();
    const root = ref.current;
    const selection = window.getSelection();
    if (!root || !selection || !selection.isCollapsed) return;

    let block: Node | null = selection.anchorNode;
    while (block && block !== root && block.parentNode !== root) {
      block = block.parentNode;
    }
    if (!block || block === root || block.nodeType !== Node.ELEMENT_NODE)
      return;
    const blockEl = block as HTMLElement;
    const tag = blockEl.tagName.toLowerCase();
    if (tag === "h1" || tag === "li") return;

    const text = blockEl.textContent ?? "";
    if (text.startsWith("# ")) {
      replaceBlockWithTag(blockEl, "h1", text.slice(2));
    } else if (text.startsWith("- ") || text.startsWith("* ")) {
      replaceBlockWithBullet(blockEl, text.slice(2));
    }
  }

  // Markdown shortcuts: typing "# ", "- " or "* " at the start of a line
  // rewrites that block into a heading or bullet — the discoverable gesture
  // everyone already tries.
  function handleKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
    if (event.key === " " || event.code === "Space") {
      applyBlockShortcut(ref.current, event);
      syncEmpty();
      return;
    }
    if (event.key === "Enter") {
      // After committing Enter, if we ended up still inside an h1 (some
      // browsers inherit the tag for the next block), demote to <p>.
      requestAnimationFrame(() => {
        const selection = window.getSelection();
        const root = ref.current;
        if (!selection || !root) return;
        let block: Node | null = selection.anchorNode;
        while (block && block.parentNode !== root) block = block.parentNode;
        if (block && (block as HTMLElement).tagName === "H1") {
          document.execCommand("formatBlock", false, "p");
        }
      });
    }
  }

  function applyFormat(command: "h1" | "bullet" | "bold") {
    const root = ref.current;
    if (!root) return;
    root.focus();
    if (command === "bold") {
      document.execCommand("bold");
    } else if (command === "bullet") {
      document.execCommand("insertUnorderedList");
    } else if (command === "h1") {
      // Toggle between h1 and paragraph for the current block.
      const selection = window.getSelection();
      if (!selection) return;
      let block: Node | null = selection.anchorNode;
      while (block && block.parentNode !== root) block = block.parentNode;
      const tag =
        (block as HTMLElement | null)?.tagName?.toLowerCase() === "h1"
          ? "p"
          : "h1";
      document.execCommand("formatBlock", false, tag);
    }
    syncEmpty();
  }

  return (
    <>
      <div
        key={noteId}
        ref={ref}
        className="note-preview"
        contentEditable
        suppressContentEditableWarning
        role="textbox"
        aria-multiline="true"
        aria-label="Generated note"
        data-placeholder={emptyPlaceholder ?? "Record or start writing…"}
        onKeyDown={handleKeyDown}
        onInput={handleInput}
        onBlur={commit}
        dangerouslySetInnerHTML={{ __html: initialHtml }}
      />
      {toolbar ? (
        <div
          className="selection-toolbar"
          role="toolbar"
          aria-label="Format selection"
          style={{ left: toolbar.x, top: toolbar.y }}
          onMouseDown={(event) => event.preventDefault()}
        >
          <button
            type="button"
            onMouseDown={(event) => event.preventDefault()}
            onClick={() => applyFormat("h1")}
            title="Heading"
            aria-label="Heading"
          >
            <IconH1 size={16} />
          </button>
          <button
            type="button"
            onMouseDown={(event) => event.preventDefault()}
            onClick={() => applyFormat("bullet")}
            title="Bullet list"
            aria-label="Bullet list"
          >
            <IconBulletList size={16} />
          </button>
          <span className="divider" aria-hidden />
          <button
            type="button"
            onMouseDown={(event) => event.preventDefault()}
            onClick={() => applyFormat("bold")}
            title="Bold"
            aria-label="Bold"
          >
            <IconBold size={16} />
          </button>
        </div>
      ) : null}
    </>
  );
}

/* ---- markdown <-> html (tiny subset: headings, lists, paragraphs, bold) -- */

function escapeHtml(value: string) {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

function inlineToHtml(text: string) {
  return escapeHtml(text)
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/(^|[^*])\*([^*]+)\*/g, "$1<em>$2</em>");
}

function markdownToHtml(markdown: string): string {
  const lines = markdown.split("\n");
  const html: string[] = [];
  let listOpen = false;

  function closeList() {
    if (listOpen) {
      html.push("</ul>");
      listOpen = false;
    }
  }

  for (const rawLine of lines) {
    const line = rawLine.trim();
    if (!line) {
      closeList();
      continue;
    }
    const heading = line.match(/^(#{1,4})\s+(.+)$/);
    if (heading) {
      closeList();
      const tag = heading[1].length === 1 ? "h1" : "h2";
      html.push(`<${tag}>${inlineToHtml(heading[2])}</${tag}>`);
      continue;
    }
    const item = line.match(/^[-*]\s+(.+)$/);
    if (item) {
      if (!listOpen) {
        html.push("<ul>");
        listOpen = true;
      }
      html.push(`<li>${inlineToHtml(item[1])}</li>`);
      continue;
    }
    closeList();
    html.push(`<p>${inlineToHtml(line)}</p>`);
  }
  closeList();
  // Leave empty notes truly empty. Seeding "<p><br></p>" sounds reasonable
  // but Chrome inserts typed characters before the trailing <br>, which
  // keeps that <br> in the paragraph — so every line you type ends with a
  // visible line break ("each space pushes to the next line"). Loose text
  // under root for the very first keystroke is fine; the markdown shortcut
  // wraps it on the first space.
  return html.join("");
}

function inlineToMarkdown(node: Node): string {
  if (node.nodeType === Node.TEXT_NODE) return node.textContent ?? "";
  if (node.nodeType !== Node.ELEMENT_NODE) return "";
  const el = node as HTMLElement;
  const inner = Array.from(el.childNodes).map(inlineToMarkdown).join("");
  const tag = el.tagName.toLowerCase();
  if (tag === "strong" || tag === "b") return `**${inner}**`;
  if (tag === "em" || tag === "i") return `*${inner}*`;
  if (tag === "br") return "\n";
  return inner;
}

/* Rewrites the caret's current line into a heading / list item when the
 * text typed so far is a markdown marker ("#", "-", "*"). */
function applyBlockShortcut(
  root: HTMLDivElement | null,
  event: React.KeyboardEvent<HTMLDivElement>,
) {
  const selection = window.getSelection();
  if (!root || !selection || !selection.isCollapsed) return;
  const range = selection.getRangeAt(0);

  // Walk up to the block element that is a direct child of the editor
  // root. The caret can land in three places we have to handle:
  //   1. inside a real block (<p>, <h1>, <li>, …) — easy case
  //   2. inside a loose text node sitting directly under root — wrap it
  //   3. on the root itself with no children — seed a <p> and move on
  let block: Node | null = range.startContainer;
  while (block && block !== root && block.parentNode !== root) {
    block = block.parentNode;
  }
  let blockEl: HTMLElement;
  if (block && block !== root && block.nodeType === Node.ELEMENT_NODE) {
    blockEl = block as HTMLElement;
  } else if (block && block !== root && block.nodeType === Node.TEXT_NODE) {
    const wrapper = document.createElement("p");
    block.parentNode?.insertBefore(wrapper, block);
    wrapper.appendChild(block);
    blockEl = wrapper;
  } else {
    // No proper block under the caret — bail rather than rewrap the entire
    // editor, which used to scramble the DOM mid-keystroke.
    return;
  }

  const pre = document.createRange();
  pre.selectNodeContents(blockEl);
  pre.setEnd(range.startContainer, range.startOffset);
  const marker = pre.toString();

  const tag =
    marker === "#"
      ? "h1"
      : marker === "-" || marker === "*"
        ? "li"
        : null;
  if (!tag) return;

  event.preventDefault();
  const rest = (blockEl.textContent ?? "").slice(marker.length);

  if (tag === "li") {
    const li = document.createElement("li");
    li.textContent = rest;
    const prev = blockEl.previousElementSibling;
    if (prev && prev.tagName === "UL") {
      prev.appendChild(li);
      blockEl.remove();
    } else {
      const ul = document.createElement("ul");
      ul.appendChild(li);
      blockEl.replaceWith(ul);
    }
    placeCaretAtEnd(li);
    return;
  }

  const heading = document.createElement(tag);
  heading.textContent = rest;
  blockEl.replaceWith(heading);
  placeCaretAtEnd(heading);
}

function replaceBlockWithTag(blockEl: HTMLElement, tag: string, text: string) {
  const heading = document.createElement(tag);
  heading.textContent = text;
  blockEl.replaceWith(heading);
  placeCaretAtEnd(heading);
}

function replaceBlockWithBullet(blockEl: HTMLElement, text: string) {
  const li = document.createElement("li");
  li.textContent = text;
  const prev = blockEl.previousElementSibling;
  if (prev && prev.tagName === "UL") {
    prev.appendChild(li);
    blockEl.remove();
  } else {
    const ul = document.createElement("ul");
    ul.appendChild(li);
    blockEl.replaceWith(ul);
  }
  placeCaretAtEnd(li);
}

function placeCaretAtEnd(el: HTMLElement) {
  const selection = window.getSelection();
  if (!selection) return;
  const range = document.createRange();
  range.selectNodeContents(el);
  range.collapse(false);
  selection.removeAllRanges();
  selection.addRange(range);
}

function htmlToMarkdown(root: HTMLElement): string {
  const blocks: string[] = [];
  for (const node of Array.from(root.childNodes)) {
    if (node.nodeType === Node.TEXT_NODE) {
      const text = node.textContent?.trim();
      if (text) blocks.push(text);
      continue;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) continue;
    const el = node as HTMLElement;
    const tag = el.tagName.toLowerCase();
    if (tag === "h1") {
      blocks.push(`# ${inlineToMarkdown(el).trim()}`);
    } else if (tag === "h2" || tag === "h3" || tag === "h4") {
      blocks.push(`## ${inlineToMarkdown(el).trim()}`);
    } else if (tag === "ul" || tag === "ol") {
      for (const li of Array.from(el.querySelectorAll("li"))) {
        blocks.push(`- ${inlineToMarkdown(li).trim()}`);
      }
    } else {
      const text = inlineToMarkdown(el).trim();
      if (text) blocks.push(text);
    }
  }
  return blocks.join("\n\n");
}
