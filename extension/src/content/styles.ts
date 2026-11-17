/** Styles for the in-page video button (inside a shadow root). Dark liquid glass. */
export const VIDEO_BUTTON_CSS = `
:host { all: initial; }
* { box-sizing: border-box; }
.wrap {
  position: fixed; top: 0; left: 0;
  width: max-content;
  font: 500 12.5px/1.2 ui-sans-serif, system-ui, -apple-system, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif;
  color: #f3f5fb;
  opacity: 0; transform: translate3d(var(--x, 0px), var(--y, 0px), 0) scale(.94);
  transform-origin: top right;
  transition: opacity .16s ease, transform .16s cubic-bezier(.2,.9,.3,1.2);
  pointer-events: none;
  -webkit-font-smoothing: antialiased;
}
.wrap.show { opacity: 1; transform: translate3d(var(--x, 0px), var(--y, 0px), 0) scale(1); }
.wrap.show .pill, .wrap.show .dismiss, .wrap.show .menu { pointer-events: auto; }
.controls { display: flex; align-items: center; gap: 4px; }
.dismiss {
  display: inline-flex; align-items: center; justify-content: center;
  width: 28px; height: 28px; padding: 0; border-radius: 50%;
  cursor: pointer; color: inherit;
}
.dismiss:hover { background: rgba(58,64,86,.9); border-color: rgba(255,255,255,.35); }
.dismiss:focus-visible { outline: 2px solid #b8c3ff; outline-offset: 2px; }
.glass {
  background: linear-gradient(180deg, rgba(40,44,58,.62), rgba(18,20,28,.70));
  -webkit-backdrop-filter: blur(18px) saturate(170%);
  backdrop-filter: blur(18px) saturate(170%);
  border: 1px solid rgba(255,255,255,.14);
  box-shadow: 0 10px 30px rgba(0,0,0,.38), 0 1px 0 rgba(255,255,255,.22) inset, 0 -1px 0 rgba(0,0,0,.25) inset;
}
.pill {
  display: inline-flex; align-items: center; gap: 7px;
  height: 32px; padding: 0 10px 0 9px; border-radius: 999px;
  cursor: pointer; color: inherit; font: inherit; letter-spacing: .01em;
  outline: none; user-select: none; white-space: nowrap;
  transition: background .15s ease, border-color .15s ease, box-shadow .15s ease;
}
.pill:hover, .pill[aria-expanded="true"] {
  border-color: rgba(255,255,255,.26);
  background: linear-gradient(180deg, rgba(58,64,86,.70), rgba(24,27,38,.78));
  box-shadow: 0 12px 34px rgba(0,0,0,.42), 0 0 0 3px rgba(120,140,255,.18), 0 1px 0 rgba(255,255,255,.28) inset;
}
.pill:focus-visible { box-shadow: 0 0 0 3px rgba(140,160,255,.55); }
.pill .label { font-weight: 600; }
.pill .chev { opacity: .7; display: inline-flex; transition: transform .15s ease; }
.pill[aria-expanded="true"] .chev { transform: rotate(180deg); }
.pill.done { border-color: rgba(79,224,163,.55); }
.pill.err { border-color: rgba(255,107,139,.6); }
.prism-mark { display: block; filter: drop-shadow(0 0 6px rgba(150,170,255,.35)); }
.menu {
  position: absolute; right: 0; top: 38px;
  width: 300px; max-height: 340px; overflow: auto;
  border-radius: 14px; padding: 6px;
  display: none;
  scrollbar-width: thin; scrollbar-color: rgba(255,255,255,.25) transparent;
}
.menu.open { display: block; animation: pop .14s ease-out; }
.menu.up { top: auto; bottom: 38px; }
@keyframes pop { from { opacity: 0; transform: translateY(-4px); } to { opacity: 1; transform: none; } }
.head { display: flex; align-items: center; gap: 8px; padding: 6px 8px 8px; border-bottom: 1px solid rgba(255,255,255,.08); margin-bottom: 4px; }
.head .t { flex: 1; min-width: 0; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.head .src { font-size: 10.5px; text-transform: uppercase; letter-spacing: .08em; color: rgba(243,245,251,.5); }
.group { padding: 8px 8px 4px; font-size: 10.5px; text-transform: uppercase; letter-spacing: .08em; color: rgba(243,245,251,.48); }
.item {
  display: flex; align-items: center; gap: 10px; width: 100%;
  padding: 8px; border: 0; border-radius: 9px; background: transparent;
  color: inherit; font: inherit; text-align: left; cursor: pointer;
}
.item:hover, .item:focus-visible { background: rgba(255,255,255,.09); outline: none; }
.item .ic { flex: none; display: inline-flex; color: #a9b6ff; }
.item .tx { flex: 1; min-width: 0; }
.item .l { display: block; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.item .d { display: block; margin-top: 2px; font-size: 11px; color: rgba(243,245,251,.55); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.item .go { opacity: 0; color: #cfd6ff; display: inline-flex; transition: opacity .12s; }
.item:hover .go { opacity: 1; }
.note { margin: 4px; padding: 8px 10px; border-radius: 9px; font-size: 11.5px; line-height: 1.4; color: #ffd9a8; background: rgba(255,170,60,.10); border: 1px solid rgba(255,170,60,.18); }
.empty, .loading { padding: 14px 10px; color: rgba(243,245,251,.65); display: flex; align-items: center; gap: 10px; }
.spin { width: 16px; height: 16px; border-radius: 50%; border: 2px solid rgba(255,255,255,.18); border-top-color: #b8c3ff; animation: r .7s linear infinite; }
@keyframes r { to { transform: rotate(360deg); } }
@media (prefers-reduced-motion: reduce) { .wrap, .menu.open { transition: none; animation: none; } }
`;
