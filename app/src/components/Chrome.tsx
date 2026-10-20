// Toolbar, tab strip, status bar and the empty-editor welcome view.

import { useEffect, useRef, useState, type MouseEvent, type ReactNode } from "react";
import type { Language, ValidationReport } from "../api";
import type { ContextObject, Tab } from "../types";
import { baseName, dirName, plural } from "../util";
import { AssayStrip } from "./BottomPanel";
import { FileIcon, fileKind } from "./FileIcon";
import {
  IconCheck,
  IconChevron,
  IconClose,
  IconExport,
  IconLink,
  IconMetamodel,
  IconModel,
  IconNew,
  IconOpen,
  IconReload,
  IconReset,
  IconRun,
  IconSave,
  IconSaveAs,
} from "./icons";

const isMac = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform);
export const MOD = isMac ? "⌘" : "Ctrl+";
const SHIFT = isMac ? "⇧" : "Shift+";

function ToolButton(props: {
  icon: ReactNode;
  label: string;
  shortcut?: string;
  onClick: (e: MouseEvent<HTMLButtonElement>) => void;
  disabled?: boolean;
  primary?: boolean;
  hideLabel?: boolean;
}) {
  const title = props.shortcut ? `${props.label} (${props.shortcut})` : props.label;
  return (
    <button
      className={`tool${props.primary ? " tool-primary" : ""}`}
      onClick={props.onClick}
      disabled={props.disabled}
      title={title}
      aria-label={props.hideLabel ? props.label : undefined}
    >
      {props.icon}
      {!props.hideLabel && <span>{props.label}</span>}
    </button>
  );
}

type ImportTarget = "oclinecore" | "hutn";

const NEW_ITEMS: { language: Language; label: string; hint: string }[] = [
  { language: "ocl", label: "Complete OCL document", hint: "constraints for loaded metamodels" },
  { language: "oclinecore", label: "Metamodel (OCLinEcore)", hint: "classes with embedded OCL" },
  { language: "hutn", label: "Model (HUTN)", hint: "objects of a metamodel" },
];

/** A button opening the menu of document kinds to create (or import as text). */
function NewMenu(props: {
  className: string;
  title: string;
  children: ReactNode;
  onNew: (language: Language) => void;
  onImport: (to: ImportTarget) => void;
}) {
  const [position, setPosition] = useState<{ top: number; left: number } | null>(null);
  const button = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!position) return;
    const close = (e: Event) => {
      const target = e.target as Node;
      if (!menu.current?.contains(target) && !button.current?.contains(target)) setPosition(null);
    };
    const escape = (e: KeyboardEvent) => {
      if (e.key === "Escape") setPosition(null);
    };
    window.addEventListener("pointerdown", close, true);
    window.addEventListener("keydown", escape);
    window.addEventListener("resize", close);
    menu.current?.querySelector("button")?.focus();
    return () => {
      window.removeEventListener("pointerdown", close, true);
      window.removeEventListener("keydown", escape);
      window.removeEventListener("resize", close);
    };
  }, [position]);

  const toggle = () => {
    if (position) {
      setPosition(null);
      return;
    }
    const r = button.current?.getBoundingClientRect();
    if (r) setPosition({ top: r.bottom + 4, left: Math.min(r.left, window.innerWidth - 300) });
  };
  const pick = (action: () => void) => () => {
    setPosition(null);
    action();
  };

  return (
    <>
      <button
        ref={button}
        className={props.className}
        onClick={toggle}
        title={props.title}
        aria-label={props.title}
        aria-haspopup="menu"
        aria-expanded={position !== null}
      >
        {props.children}
      </button>
      {position && (
        // Fixed, so that scrolling containers (the tab strip) do not clip it.
        <div ref={menu} className="menu" role="menu" style={{ top: position.top, left: position.left }}>
          {NEW_ITEMS.map((i) => (
            <button key={i.language} role="menuitem" className="menu-item" onClick={pick(() => props.onNew(i.language))}>
              <span className="menu-label">
                <FileIcon kind={i.language} />
                {i.label}
              </span>
              <span className="menu-hint">.{i.language}</span>
            </button>
          ))}
          <div className="menu-sep" role="separator" />
          <button role="menuitem" className="menu-item" onClick={pick(() => props.onImport("oclinecore"))}>
            <span className="menu-label">
              <FileIcon kind="ecore" />
              Metamodel from .ecore file…
            </span>
            <span className="menu-hint">as OCLinEcore</span>
          </button>
          <button role="menuitem" className="menu-item" onClick={pick(() => props.onImport("hutn"))}>
            <span className="menu-label">
              <FileIcon kind="xmi" />
              Model from XMI file…
            </span>
            <span className="menu-hint">as HUTN</span>
          </button>
        </div>
      )}
    </>
  );
}

export interface ToolbarProps {
  canSave: boolean;
  canExport: boolean;
  exportLabel: string;
  /** Export opens a menu (of export options). */
  exportMenu: boolean;
  busy: boolean;
  onNew: (language: Language) => void;
  onImport: (to: ImportTarget) => void;
  onOpen: () => void;
  onSave: () => void;
  onSaveAs: () => void;
  onExport: (anchor: HTMLElement) => void;
  onLoadMetamodel: () => void;
  onLoadModel: () => void;
  onReload: () => void;
  onMapUri: () => void;
  onCheck: () => void;
  onRun: () => void;
  onReset: () => void;
}

export function Toolbar(p: ToolbarProps) {
  return (
    <header className="toolbar">
      <div className="brand" aria-label="Oxychloride">
        <svg width="18" height="18" viewBox="0 0 16 16" aria-hidden="true">
          <path d="M8 1 14.06 4.5v7L8 15 1.94 11.5v-7z" fill="var(--accent)" />
          <path d="M8 4.4 10.9 6.1v3.4L8 11.2 5.1 9.5V6.1z" fill="var(--bg)" />
        </svg>
        <span>Oxychloride</span>
      </div>
      <div className="tool-group" role="group" aria-label="File">
        <NewMenu className="tool tool-menu" title={`New document (${MOD}N: like the current one)`} onNew={p.onNew} onImport={p.onImport}>
          <IconNew />
          <IconChevron className="menu-chevron" />
        </NewMenu>
        <ToolButton icon={<IconOpen />} label="Open…" shortcut={`${MOD}O`} onClick={p.onOpen} hideLabel />
        <ToolButton icon={<IconSave />} label="Save" shortcut={`${MOD}S`} onClick={p.onSave} disabled={!p.canSave} hideLabel />
        <ToolButton
          icon={<IconSaveAs />}
          label="Save As…"
          shortcut={`${SHIFT}${MOD}S`}
          onClick={p.onSaveAs}
          disabled={!p.canSave}
          hideLabel
        />
        <ToolButton
          icon={
            <>
              <IconExport />
              {p.exportMenu && <IconChevron className="menu-chevron" />}
            </>
          }
          label={p.exportLabel}
          onClick={(e) => p.onExport(e.currentTarget)}
          disabled={!p.canExport}
          hideLabel
        />
      </div>
      <div className="tool-group" role="group" aria-label="Workspace">
        <ToolButton icon={<IconMetamodel />} label="Load metamodel" onClick={p.onLoadMetamodel} />
        <ToolButton icon={<IconModel />} label="Load model" onClick={p.onLoadModel} />
        <ToolButton icon={<IconLink />} label="Map URI prefix…" onClick={p.onMapUri} hideLabel />
        <ToolButton icon={<IconReload />} label="Reload metamodels and models" onClick={p.onReload} hideLabel />
      </div>
      <div className="tool-group" role="group" aria-label="Run">
        <ToolButton icon={<IconCheck />} label="Check" shortcut={`${SHIFT}${MOD}↩`} onClick={p.onCheck} />
        <ToolButton icon={<IconRun />} label="Run" shortcut={`${MOD}↩`} onClick={p.onRun} disabled={p.busy} primary />
      </div>
      <div className="toolbar-spacer" />
      <ToolButton icon={<IconReset />} label="Reset workspace" onClick={p.onReset} hideLabel />
    </header>
  );
}

const LANGUAGE_TITLES: Record<Language, string> = {
  ocl: "Complete OCL document",
  oclinecore: "Metamodel (OCLinEcore)",
  hutn: "Model (HUTN)",
  xmi: "Model (XMI)",
};

export function TabBar(props: {
  tabs: Tab[];
  activeId: string | null;
  onSelect: (id: string) => void;
  onClose: (id: string) => void;
  onNew: (language: Language) => void;
  onImport: (to: ImportTarget) => void;
  errorCounts: Map<string, number>;
}) {
  const { tabs, activeId, onSelect, onClose, onNew, onImport, errorCounts } = props;
  return (
    <div className="tabbar" role="tablist" aria-label="Open documents">
      {tabs.map((t) => {
        const errors = errorCounts.get(t.id) ?? 0;
        return (
          <div
            key={t.id}
            role="tab"
            aria-selected={t.id === activeId}
            tabIndex={0}
            className={`doc-tab${t.id === activeId ? " is-active" : ""}${t.dirty ? " is-dirty" : ""}`}
            title={t.path ?? t.name}
            onClick={() => onSelect(t.id)}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " ") onSelect(t.id);
            }}
            onAuxClick={(e) => {
              if (e.button === 1) onClose(t.id);
            }}
          >
            <FileIcon kind={t.language} title={LANGUAGE_TITLES[t.language]} />
            <span className={`doc-title${errors ? " has-errors" : ""}`}>{t.title}</span>
            <button
              className="doc-close"
              aria-label={t.dirty ? `Close ${t.title} (unsaved changes)` : `Close ${t.title}`}
              onClick={(e) => {
                e.stopPropagation();
                onClose(t.id);
              }}
            >
              <span className="dirty-dot" aria-hidden="true" />
              <IconClose />
            </button>
          </div>
        );
      })}
      <NewMenu className="tab-new" title="New document" onNew={onNew} onImport={onImport}>
        <IconNew />
      </NewMenu>
    </div>
  );
}

export function Welcome(props: {
  recent: { ocl: string[]; ecore: string[]; model: string[] };
  onNew: (language: Language) => void;
  onImport: (to: ImportTarget) => void;
  onOpen: () => void;
  onOpenPath: (path: string) => void;
  onLoadMetamodelPath: (path: string) => void;
  onLoadModelPath: (path: string) => void;
}) {
  const { recent } = props;
  const list = (paths: string[], onPick: (p: string) => void) => (
    <ul className="recent-list">
      {paths.map((p) => {
        const kind = fileKind(p);
        return (
        <li key={p}>
          {kind && <FileIcon kind={kind} />}
          <button className="link" onClick={() => onPick(p)} title={p}>
            {baseName(p)}
          </button>
          <span className="dim">{dirName(p)}</span>
        </li>
        );
      })}
    </ul>
  );
  return (
    <div className="welcome">
      <div className="welcome-inner">
        <h1>Write constraints, check them against real models.</h1>
        <p>
          Write a metamodel in <strong>OCLinEcore</strong> — classes with invariants, derived properties, operation
          bodies and pre- and postconditions — and models of it in <strong>HUTN</strong>. Both editors complete what
          the metamodel allows, and every model is checked against its constraints as you type.
        </p>
        <p>
          Or constrain existing <code>.ecore</code> metamodels with a <strong>Complete OCL</strong> document and validate
          XMI models against it; <code>import</code> statements load metamodels relative to the document.
        </p>
        <div className="welcome-actions">
          <button className="btn btn-primary" onClick={() => props.onNew("oclinecore")}>
            New metamodel
          </button>
          <button className="btn" onClick={() => props.onNew("hutn")}>
            New model
          </button>
          <button className="btn" onClick={() => props.onNew("ocl")}>
            New OCL document
          </button>
          <button className="btn" onClick={props.onOpen}>
            Open…
          </button>
        </div>
        <p className="welcome-more">
          Converting existing files:{" "}
          <button className="link" onClick={() => props.onImport("oclinecore")}>
            .ecore to OCLinEcore
          </button>
          ,{" "}
          <button className="link" onClick={() => props.onImport("hutn")}>
            XMI to HUTN
          </button>
          .
        </p>
        {recent.ocl.length > 0 && (
          <section>
            <h2>Recent documents</h2>
            {list(recent.ocl, props.onOpenPath)}
          </section>
        )}
        {recent.ecore.length > 0 && (
          <section>
            <h2>Recent metamodels</h2>
            {list(recent.ecore, props.onLoadMetamodelPath)}
          </section>
        )}
        {recent.model.length > 0 && (
          <section>
            <h2>Recent models</h2>
            {list(recent.model, props.onLoadModelPath)}
          </section>
        )}
      </div>
    </div>
  );
}

export function StatusBar(props: {
  engine: string;
  packageCount: number;
  metamodelFiles: number;
  modelCount: number;
  objectCount: number;
  context: ContextObject | null;
  errors: number;
  warnings: number;
  report: ValidationReport | null;
  message: { text: string; level: "info" | "error" } | null;
  onProblems: () => void;
  onValidation: () => void;
  onConsole: () => void;
}) {
  const p = props;
  return (
    <footer className="statusbar">
      <span className="status-item" title="Engine">
        {p.engine}
      </span>
      <span className="status-item">
        {p.packageCount ? `${plural(p.packageCount, "package")} from ${plural(p.metamodelFiles, "file")}` : "No metamodel"}
      </span>
      <span className="status-item">
        {p.modelCount ? `${plural(p.modelCount, "model")}, ${plural(p.objectCount, "object")}` : "No model"}
      </span>
      <button className="status-item status-button" onClick={p.onConsole} title={p.context ? `${p.context.class} ${p.context.fragment}` : undefined}>
        {p.context ? `self = ${p.context.label}` : "No context object"}
      </button>
      {p.message && <span className={`status-item status-message sev-${p.message.level}`}>{p.message.text}</span>}
      <span className="status-spacer" />
      {p.report && (
        <button className="status-item status-button" onClick={p.onValidation} title="Last validation run">
          <AssayStrip summary={p.report.summary} compact />
          {p.report.summary.violated + p.report.summary.errors === 0
            ? "Valid"
            : `${p.report.summary.violated + p.report.summary.errors} failing`}
        </button>
      )}
      <button className="status-item status-button" onClick={p.onProblems}>
        <span className={p.errors ? "sev-error" : "dim"}>{plural(p.errors, "error")}</span>
        <span className={p.warnings ? "sev-warning" : "dim"}>{plural(p.warnings, "warning")}</span>
      </button>
    </footer>
  );
}
