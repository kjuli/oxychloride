// A context menu at the pointer (or below the focused element, when opened from the keyboard).

import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { IconCheck } from "./icons";

export type MenuEntry =
  | {
      kind: "item";
      label: string;
      hint?: string;
      disabled?: boolean;
      /** An option: shown with a check mark when set. */
      checked?: boolean;
      icon?: ReactNode;
      onSelect: () => void;
    }
  | { kind: "header"; label: string }
  | { kind: "separator" };

export interface MenuRequest {
  x: number;
  y: number;
  entries: MenuEntry[];
}

/** Where a context menu opened by `e` goes: the pointer, or the target's corner (keyboard). */
export function menuPoint(e: { clientX: number; clientY: number; currentTarget: Element }): { x: number; y: number } {
  if (e.clientX !== 0 || e.clientY !== 0) return { x: e.clientX, y: e.clientY };
  const r = e.currentTarget.getBoundingClientRect();
  return { x: r.left + 24, y: r.bottom };
}

export function ContextMenu({ request, onClose }: { request: MenuRequest; onClose: () => void }) {
  const menu = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState({ top: request.y, left: request.x });

  // Keep the menu inside the window.
  useLayoutEffect(() => {
    const r = menu.current?.getBoundingClientRect();
    if (!r) return;
    setPosition({
      top: Math.max(4, Math.min(request.y, window.innerHeight - r.height - 4)),
      left: Math.max(4, Math.min(request.x, window.innerWidth - r.width - 4)),
    });
  }, [request]);

  useEffect(() => {
    const close = (e: Event) => {
      if (!menu.current?.contains(e.target as Node)) onClose();
    };
    const escape = (e: globalThis.KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("pointerdown", close, true);
    window.addEventListener("contextmenu", close, true);
    window.addEventListener("keydown", escape);
    window.addEventListener("resize", onClose);
    window.addEventListener("blur", onClose);
    menu.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
    return () => {
      window.removeEventListener("pointerdown", close, true);
      window.removeEventListener("contextmenu", close, true);
      window.removeEventListener("keydown", escape);
      window.removeEventListener("resize", onClose);
      window.removeEventListener("blur", onClose);
    };
  }, [request, onClose]);

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const items = Array.from(menu.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? []);
    const i = items.indexOf(document.activeElement as HTMLButtonElement);
    const next = e.key === "ArrowDown" ? (i + 1) % items.length : (i - 1 + items.length) % items.length;
    items[next]?.focus();
  };

  return (
    <div
      ref={menu}
      className="menu context-menu"
      role="menu"
      style={position}
      onKeyDown={onKeyDown}
      onContextMenu={(e) => e.preventDefault()}
    >
      {request.entries.map((entry, i) =>
        entry.kind === "separator" ? (
          <div key={i} className="menu-sep" role="separator" />
        ) : entry.kind === "header" ? (
          <div key={i} className="menu-header" role="presentation">
            {entry.label}
          </div>
        ) : (
          <button
            key={i}
            role={entry.checked === undefined ? "menuitem" : "menuitemcheckbox"}
            aria-checked={entry.checked}
            className="menu-item"
            disabled={entry.disabled}
            onClick={() => {
              onClose();
              entry.onSelect();
            }}
          >
            <span className="menu-label">
              {entry.checked !== undefined && <span className="menu-check">{entry.checked && <IconCheck />}</span>}
              {entry.icon}
              {entry.label}
            </span>
            {entry.hint && <span className="menu-hint">{entry.hint}</span>}
          </button>
        ),
      )}
    </div>
  );
}
