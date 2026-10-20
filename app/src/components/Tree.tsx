import { useState, type KeyboardEvent, type MouseEvent, type ReactNode } from "react";
import { IconChevron } from "./icons";

export interface TreeItemProps {
  /** Indentation level (0 = top). */
  depth: number;
  label: ReactNode;
  /** Secondary text shown after the label, dimmed. */
  detail?: ReactNode;
  /** A small leading badge (e.g. the classifier kind). */
  badge?: ReactNode;
  title?: string;
  defaultOpen?: boolean;
  selected?: boolean;
  /** Called on click / Enter. Rows without children toggle nothing. */
  onSelect?: () => void;
  /** Called on double-click. */
  onOpen?: () => void;
  /** Called on right-click (or the context menu key) instead of the browser's menu. */
  onContextMenu?: (e: MouseEvent<HTMLDivElement>) => void;
  /** Children are rendered lazily, only while the item is open. */
  children?: () => ReactNode;
  hasChildren?: boolean;
  className?: string;
}

export function TreeItem(props: TreeItemProps) {
  const { depth, label, detail, badge, title, defaultOpen, selected, onSelect, onOpen, onContextMenu, children } = props;
  const { className } = props;
  const expandable = props.hasChildren ?? Boolean(children);
  const [open, setOpen] = useState(Boolean(defaultOpen));

  const activate = () => {
    if (onSelect) onSelect();
    else if (expandable) setOpen((o) => !o);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      activate();
    } else if (e.key === "ArrowRight" && expandable && !open) {
      e.preventDefault();
      setOpen(true);
    } else if (e.key === "ArrowLeft" && expandable && open) {
      e.preventDefault();
      setOpen(false);
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const rows = Array.from(
        e.currentTarget.closest("[role=tree]")?.querySelectorAll<HTMLElement>("[role=treeitem]") ?? [],
      );
      const i = rows.indexOf(e.currentTarget);
      rows[e.key === "ArrowDown" ? i + 1 : i - 1]?.focus();
    }
  };

  return (
    <>
      <div
        role="treeitem"
        tabIndex={0}
        aria-expanded={expandable ? open : undefined}
        aria-selected={selected || undefined}
        className={`tree-row${selected ? " is-selected" : ""}${className ? ` ${className}` : ""}`}
        style={{ paddingLeft: 6 + depth * 14 }}
        title={title}
        onClick={activate}
        onDoubleClick={onOpen}
        onContextMenu={
          onContextMenu &&
          ((e) => {
            e.preventDefault();
            e.currentTarget.focus();
            onContextMenu(e);
          })
        }
        onKeyDown={onKeyDown}
      >
        {expandable ? (
          <span
            className={`twisty${open ? " is-open" : ""}`}
            onClick={(e) => {
              e.stopPropagation();
              setOpen((o) => !o);
            }}
          >
            <IconChevron />
          </span>
        ) : (
          <span className="twisty" />
        )}
        {badge}
        <span className="tree-label">{label}</span>
        {detail != null && <span className="tree-detail">{detail}</span>}
      </div>
      {expandable && open && children?.()}
    </>
  );
}

/** A read-only block of OCL text inside a tree. */
export function TreeCode({ depth, prefix, text }: { depth: number; prefix: string; text: string }) {
  return (
    <div className="tree-code" style={{ marginLeft: 26 + depth * 14 }}>
      <span className="tree-code-prefix">{prefix}</span>
      <code>{text}</code>
    </div>
  );
}
