import { createRoot } from "react-dom/client";
import "./design/fonts.css";
import "./design/tokens.css";
import "./design/base.css";
import "./design/components.css";
import "./styles/shell.css";
import "./styles/main.css";
import "./styles/windows.css";
import "./styles/sheets.css";
import { App } from "./App";

const NOT_TEXT = new Set(["button", "checkbox", "color", "file", "hidden", "image", "radio", "range", "reset", "submit"]);

/** A text field (Cut, Copy, Paste) or selected text under the pointer (Copy). */
function wantsWebMenu(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;
  const field = target.closest("input, textarea, [contenteditable]");
  if (field instanceof HTMLInputElement) return !NOT_TEXT.has(field.type);
  if (field instanceof HTMLTextAreaElement || (field instanceof HTMLElement && field.isContentEditable)) return true;
  const sel = window.getSelection();
  return !!sel && !sel.isCollapsed && sel.toString().trim() !== "" && sel.containsNode(target, true);
}

// The webview's own menu (Back, Refresh, Save as, Print…) is not Prism's: every window keeps it
// only where it helps (#62). Prism's own menus (list rows, sidebar) open from their elements.
// Dev builds: Shift + right-click still opens it, for Inspect.
window.addEventListener("contextmenu", (e) => {
  if (import.meta.env.DEV && e.shiftKey) return;
  if (!wantsWebMenu(e.target)) e.preventDefault();
});

createRoot(document.getElementById("root")!).render(<App />);
