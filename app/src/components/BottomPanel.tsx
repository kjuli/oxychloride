import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import type {
  ConstraintInfo,
  ConstraintKind,
  ContractResult,
  Diagnostic,
  ObjectSummary,
  Range,
  ValidationReport,
  ValidationSummary,
} from "../api";
import type { ConsoleEntry, ContextObject } from "../types";
import { baseName, plural } from "../util";
import {
  GlyphError,
  GlyphSatisfied,
  GlyphViolated,
  IconClose,
  IconExport,
  IconRun,
  IconTarget,
  SeverityGlyph,
} from "./icons";

export type PanelTab = "problems" | "validation" | "console";

// ---------------------------------------------------------------- shared

function location(d: Diagnostic): string {
  return `${d.range.start.line + 1}:${d.range.start.column + 1}`;
}

function sourceLabel(d: Diagnostic): string {
  if (d.origin.kind === "document") return baseName(d.origin.name);
  if (d.origin.kind === "file") return baseName(d.origin.path);
  return d.source;
}

const KIND_BADGES: Record<ConstraintKind, { text: string; title: string }> = {
  invariant: { text: "inv", title: "invariant" },
  precondition: { text: "pre", title: "precondition" },
  postcondition: { text: "post", title: "postcondition" },
};

function KindBadge({ kind }: { kind: ConstraintKind }) {
  return (
    <span className={`kind-badge kind-${kind}`} title={KIND_BADGES[kind].title}>
      {KIND_BADGES[kind].text}
    </span>
  );
}

/** `Course::hasSeat()` for contracts, the class for invariants. */
function constraintContext(c: ConstraintInfo): string {
  if (!c.operation) return c.context;
  const op = c.operation.slice(0, c.operation.lastIndexOf(")") + 1) || c.operation;
  return `${c.context}::${op}`;
}

/** Proportions of satisfied / violated / failed evaluations, as one horizontal bar. */
export function AssayStrip({ summary, compact }: { summary: ValidationSummary; compact?: boolean }) {
  const total = summary.satisfied + summary.violated + summary.errors;
  const pct = (n: number) => (total ? `${(n / total) * 100}%` : "0%");
  return (
    <span
      className={`assay${compact ? " assay-compact" : ""}`}
      role="img"
      aria-label={`${summary.satisfied} satisfied, ${summary.violated} violated, ${summary.errors} errors`}
    >
      {total === 0 ? (
        <span className="assay-empty" />
      ) : (
        <>
          <span className="assay-seg st-violated" style={{ width: pct(summary.violated) }} />
          <span className="assay-seg st-error" style={{ width: pct(summary.errors) }} />
          <span className="assay-seg st-satisfied" style={{ width: pct(summary.satisfied) }} />
        </>
      )}
    </span>
  );
}

// ---------------------------------------------------------------- problems

export function ProblemsView({ problems, onJump }: { problems: Diagnostic[]; onJump: (d: Diagnostic) => void }) {
  const sorted = useMemo(() => {
    const rank = { error: 0, warning: 1, info: 2 } as const;
    return [...problems].sort((a, b) => rank[a.severity] - rank[b.severity]);
  }, [problems]);

  if (sorted.length === 0) {
    return <p className="panel-empty">No problems. Documents and the OCL embedded in loaded metamodels check cleanly.</p>;
  }
  return (
    <div className="panel-scroll">
      <table className="grid">
        <tbody>
          {sorted.map((d, i) => {
            const jumpable = d.origin.kind === "document";
            return (
              <tr
                key={i}
                className={jumpable ? "is-clickable" : undefined}
                onClick={jumpable ? () => onJump(d) : undefined}
                title={d.source}
              >
                <td className={`col-icon sev-${d.severity}`}>
                  <SeverityGlyph severity={d.severity} />
                </td>
                <td className="col-message">{d.message}</td>
                <td className="col-source">
                  {sourceLabel(d)}
                  {d.origin.kind === "ecore" && <span className="dim"> {d.origin.element}</span>}
                </td>
                <td className="col-loc">{d.origin.kind === "document" ? location(d) : ""}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

// ---------------------------------------------------------------- validation

export interface ValidationOptions {
  includeSatisfied: boolean;
  includeEcore: boolean;
  /** Check pre- and postconditions of operations. */
  contracts: boolean;
}

export interface ValidationViewProps {
  report: ValidationReport | null;
  running: boolean;
  options: ValidationOptions;
  onOptions: (o: ValidationOptions) => void;
  onRun: () => void;
  onJumpConstraint: (c: ConstraintInfo) => void;
  onJumpDiagnostic: (d: Diagnostic) => void;
  onJumpObject: (o: { resource: string; range?: Range }) => void;
  onSelectObject: (o: ObjectSummary) => void;
  context: ContextObject | null;
}

const MAX_ROWS = 2000;

function summaryText(s: ValidationSummary): string {
  const outcome =
    s.violated + s.errors === 0
      ? `All ${plural(s.evaluations, "check")} passed`
      : [
          s.violated ? `${s.violated} violated` : null,
          s.errors ? plural(s.errors, "error") : null,
          `${s.satisfied} satisfied`,
        ]
          .filter(Boolean)
          .join(", ");
  return `${outcome}. ${plural(s.constraints, "constraint")} on ${plural(s.objects, "object")}.`;
}

export function ValidationView(props: ValidationViewProps) {
  const { report, running, options, onOptions, onRun, onJumpConstraint, onJumpDiagnostic, onJumpObject, onSelectObject, context } =
    props;

  const toolbar = (
    <div className="panel-toolbar">
      <button className="btn btn-primary btn-small" onClick={onRun} disabled={running}>
        <IconRun /> {running ? "Running…" : "Run validation"}
      </button>
      <label className="check">
        <input
          type="checkbox"
          checked={options.includeSatisfied}
          onChange={(e) => onOptions({ ...options, includeSatisfied: e.target.checked })}
        />
        Show satisfied
      </label>
      <label className="check">
        <input
          type="checkbox"
          checked={options.includeEcore}
          onChange={(e) => onOptions({ ...options, includeEcore: e.target.checked })}
        />
        Include metamodel constraints
      </label>
      <label className="check" title="Check the pre- and postconditions of operations, at every call and for every object">
        <input
          type="checkbox"
          checked={options.contracts}
          onChange={(e) => onOptions({ ...options, contracts: e.target.checked })}
        />
        Pre/postconditions
      </label>
      {report && (
        <span className="validation-summary">
          <AssayStrip summary={report.summary} />
          <span>{summaryText(report.summary)}</span>
        </span>
      )}
    </div>
  );

  if (!report) {
    return (
      <div className="panel-fill">
        {toolbar}
        <p className="panel-empty">
          Run validation to check every invariant from the open documents
          {options.includeEcore ? " and the metamodels" : ""} against every object of the loaded models
          {options.contracts ? ", and the pre- and postconditions of their operations" : ""}.
        </p>
      </div>
    );
  }

  const rows = report.results.slice(0, MAX_ROWS);
  return (
    <div className="panel-fill">
      {toolbar}
      <div className="panel-scroll">
        {report.diagnostics.length > 0 && (
          <div className="compile-problems">
            {report.diagnostics.map((d, i) => (
              <div
                key={i}
                className={`compile-problem sev-${d.severity}${d.origin.kind === "document" ? " is-clickable" : ""}`}
                onClick={d.origin.kind === "document" ? () => onJumpDiagnostic(d) : undefined}
              >
                <SeverityGlyph severity={d.severity} />
                <span>{d.message}</span>
                <span className="dim">{sourceLabel(d)}</span>
              </div>
            ))}
          </div>
        )}
        {report.notes && report.notes.length > 0 && (
          <div className="compile-problems">
            {report.notes.map((n, i) => (
              <div key={i} className="compile-problem sev-info">
                <SeverityGlyph severity="info" />
                <span>{n}</span>
              </div>
            ))}
          </div>
        )}
        {rows.length === 0 ? (
          <p className="panel-empty">
            {report.summary.evaluations === 0
              ? "Nothing was evaluated: load a model, and open a document with invariants for its classes."
              : "Every constraint is satisfied. Turn on Show satisfied to list each check."}
          </p>
        ) : (
          <table className="grid validation-grid">
            <thead>
              <tr>
                <th className="col-icon" aria-label="Status" />
                <th>Constraint</th>
                <th>Context</th>
                <th>Object</th>
                <th>Message</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((r, i) => {
                const c = report.constraints[r.constraint];
                const isContext =
                  context?.resource === r.object.resource && context.fragment === r.object.fragment;
                return (
                  <tr key={i} className={`st-row-${r.status}`}>
                    <td className={`col-icon st-${r.status}`} title={r.status}>
                      {r.status === "satisfied" ? <GlyphSatisfied /> : r.status === "violated" ? <GlyphViolated /> : <GlyphError />}
                    </td>
                    <td>
                      {c ? (
                        <>
                          <KindBadge kind={c.kind} />
                          <button className="link mono" onClick={() => onJumpConstraint(c)} title={c.text}>
                            {c.name}
                          </button>
                        </>
                      ) : (
                        "?"
                      )}
                    </td>
                    <td className="mono dim">{c ? constraintContext(c) : ""}</td>
                    <td>
                      <button
                        className={`link object-link${isContext ? " is-context" : ""}`}
                        onClick={() => onSelectObject(r.object)}
                        title={`${r.object.class} ${r.object.fragment}\nUse as evaluation context`}
                      >
                        {r.object.label}
                      </button>
                      {r.object.range && (
                        <button
                          className="icon-link"
                          onClick={() => onJumpObject(r.object)}
                          title="Show the object's definition"
                          aria-label={`Show the definition of ${r.object.label}`}
                        >
                          <IconExport />
                        </button>
                      )}
                    </td>
                    <td className="col-message">
                      {r.call && <span className="mono call">{r.call}</span>}
                      {r.call && r.message ? ": " : ""}
                      {r.message ?? ""}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
        {report.results.length > MAX_ROWS && (
          <p className="panel-note">Showing the first {MAX_ROWS} of {report.results.length} results.</p>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------- console

export interface ConsoleViewProps {
  entries: ConsoleEntry[];
  context: ContextObject | null;
  onEvaluate: (expression: string) => void;
  onClear: () => void;
  onClearContext: () => void;
  onJumpDiagnostic: (d: Diagnostic) => void;
  onJumpConstraint: (c: ConstraintInfo) => void;
}

function ContractLine({ c, onJump }: { c: ContractResult; onJump: (c: ConstraintInfo) => void }) {
  const jumpable = c.constraint.origin.kind === "document";
  return (
    <div
      className={`console-diag sev-${c.status === "violated" ? "error" : "warning"}${jumpable ? " is-clickable" : ""}`}
      onClick={jumpable ? () => onJump(c.constraint) : undefined}
      title={c.constraint.text}
    >
      <SeverityGlyph severity={c.status === "violated" ? "error" : "warning"} />
      <span>
        {c.constraint.kind} {c.constraint.name !== "<anonymous>" ? `${c.constraint.name} ` : ""}of{" "}
        {constraintContext(c.constraint)} {c.status === "violated" ? "violated" : "failed"} by{" "}
        <code>{c.call}</code> on {c.object.label}
        {c.message ? `: ${c.message}` : ""}
      </span>
    </div>
  );
}

function EvaluationOutput({
  entry,
  onJumpConstraint,
}: {
  entry: Extract<ConsoleEntry, { kind: "evaluation" }>;
  onJumpConstraint: (c: ConstraintInfo) => void;
}): ReactNode {
  if (entry.error) return <div className="console-out sev-error">{entry.error}</div>;
  if (!entry.result) return <div className="console-out dim">Evaluating…</div>;
  const r = entry.result;
  const failed = r.diagnostics.some((d) => d.severity === "error");
  return (
    <>
      {!failed && (
        <div className={`console-out${r.invalid ? " is-invalid" : ""}`}>
          <span className="console-value">{r.display}</span>
          {r.type && <span className="console-type">{r.type}</span>}
        </div>
      )}
      {r.diagnostics.map((d, i) => (
        <div key={i} className={`console-diag sev-${d.severity}`}>
          <SeverityGlyph severity={d.severity} />
          <span>
            {d.range.start.line === 0 && d.origin.kind === "expression" ? `col ${d.range.start.column + 1}: ` : ""}
            {d.message}
          </span>
        </div>
      ))}
      {r.contracts?.map((c, i) => <ContractLine key={`c${i}`} c={c} onJump={onJumpConstraint} />)}
    </>
  );
}

export function ConsoleView(props: ConsoleViewProps) {
  const { entries, context, onEvaluate, onClear, onClearContext, onJumpDiagnostic, onJumpConstraint } = props;
  const [input, setInput] = useState("");
  const [historyIndex, setHistoryIndex] = useState<number | null>(null);
  const draft = useRef("");
  const scroller = useRef<HTMLDivElement>(null);

  const history = useMemo(
    () => entries.flatMap((e) => (e.kind === "evaluation" ? [e.expression] : [])).filter((x, i, a) => a[i - 1] !== x),
    [entries],
  );

  useEffect(() => {
    const el = scroller.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [entries]);

  const submit = () => {
    const expression = input.trim();
    if (!expression) return;
    onEvaluate(expression);
    setInput("");
    setHistoryIndex(null);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") {
      e.preventDefault();
      submit();
    } else if (e.key === "ArrowUp" && history.length) {
      e.preventDefault();
      const next = historyIndex === null ? history.length - 1 : Math.max(0, historyIndex - 1);
      if (historyIndex === null) draft.current = input;
      setHistoryIndex(next);
      setInput(history[next]);
    } else if (e.key === "ArrowDown" && historyIndex !== null) {
      e.preventDefault();
      const next = historyIndex + 1;
      if (next >= history.length) {
        setHistoryIndex(null);
        setInput(draft.current);
      } else {
        setHistoryIndex(next);
        setInput(history[next]);
      }
    }
  };

  return (
    <div className="panel-fill console">
      <div className="panel-scroll" ref={scroller}>
        {entries.length === 0 && (
          <p className="panel-empty">
            Evaluate an OCL expression against the selected model object, for example{" "}
            <code>self.oclType()</code> or <code>Book.allInstances()-&gt;size()</code>. Pick a context object in the
            Models tree.
          </p>
        )}
        {entries.map((e) =>
          e.kind === "note" ? (
            <div key={e.id} className={`console-note sev-${e.level}`}>
              <span>{e.text}</span>
              {e.diagnostics?.map((d, i) => (
                <div
                  key={i}
                  className={`console-diag sev-${d.severity}${d.origin.kind === "document" ? " is-clickable" : ""}`}
                  onClick={d.origin.kind === "document" ? () => onJumpDiagnostic(d) : undefined}
                >
                  <SeverityGlyph severity={d.severity} />
                  <span>{d.message}</span>
                </div>
              ))}
            </div>
          ) : (
            <div key={e.id} className="console-entry">
              <div className="console-in">
                <span className="console-prompt" aria-hidden="true">›</span>
                <code>{e.expression}</code>
                <span className="console-ctx">{e.context ? `self = ${e.context.label}` : "no context"}</span>
              </div>
              <EvaluationOutput entry={e} onJumpConstraint={onJumpConstraint} />
            </div>
          ),
        )}
      </div>
      <div className="console-bar">
        <span className={`context-chip${context ? "" : " is-empty"}`} title={context ? `${context.class}\n${context.fragment}` : undefined}>
          <IconTarget />
          {context ? (
            <>
              <span className="context-label">{context.label}</span>
              <button className="chip-clear" onClick={onClearContext} aria-label="Clear evaluation context">
                <IconClose />
              </button>
            </>
          ) : (
            <span>No context</span>
          )}
        </span>
        <input
          className="console-input"
          value={input}
          onChange={(e) => {
            setInput(e.target.value);
            setHistoryIndex(null);
          }}
          onKeyDown={onKeyDown}
          placeholder="OCL expression, Enter to evaluate"
          spellCheck={false}
          autoCapitalize="off"
          autoCorrect="off"
          aria-label="OCL expression"
        />
        <button className="btn btn-small" onClick={submit} disabled={!input.trim()}>
          Evaluate
        </button>
        <button className="btn btn-small btn-quiet" onClick={onClear} disabled={entries.length === 0}>
          Clear
        </button>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------- panel frame

export interface BottomPanelProps {
  tab: PanelTab;
  onTab: (t: PanelTab) => void;
  counts: { errors: number; warnings: number };
  report: ValidationReport | null;
  children: ReactNode;
}

export function BottomPanel({ tab, onTab, counts, report, children }: BottomPanelProps) {
  const tabs: { id: PanelTab; label: string; badge?: ReactNode }[] = [
    {
      id: "problems",
      label: "Problems",
      badge:
        counts.errors + counts.warnings > 0 ? (
          <span className={`tab-count ${counts.errors ? "sev-error" : "sev-warning"}`}>{counts.errors + counts.warnings}</span>
        ) : undefined,
    },
    {
      id: "validation",
      label: "Validation",
      badge:
        report && report.summary.violated + report.summary.errors > 0 ? (
          <span className="tab-count st-violated">{report.summary.violated + report.summary.errors}</span>
        ) : report ? (
          <span className="tab-count st-satisfied">✓</span>
        ) : undefined,
    },
    { id: "console", label: "Console" },
  ];
  return (
    <section className="panel">
      <div className="panel-tabs" role="tablist">
        {tabs.map((t) => (
          <button
            key={t.id}
            role="tab"
            aria-selected={tab === t.id}
            className={`panel-tab${tab === t.id ? " is-active" : ""}`}
            onClick={() => onTab(t.id)}
          >
            {t.label}
            {t.badge}
          </button>
        ))}
      </div>
      <div className="panel-body" role="tabpanel">
        {children}
      </div>
    </section>
  );
}
