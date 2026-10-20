import { confirm, message as messageDialog, open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";

import {
  engineAvailable,
  errorMessage,
  languageOf,
  readTextFile,
  rpc,
  writeTextFile,
  type ConstraintInfo,
  type Diagnostic,
  type Language,
  type LoadResult,
  type ModelResourceInfo,
  type ObjectInfo,
  type ObjectSummary,
  type PackageInfo,
  type Range,
  type ValidationReport,
} from "./api";
import {
  BottomPanel,
  ConsoleView,
  ProblemsView,
  ValidationView,
  type PanelTab,
  type ValidationOptions,
} from "./components/BottomPanel";
import { StatusBar, TabBar, Toolbar, Welcome } from "./components/Chrome";
import { ContextMenu, menuPoint, type MenuEntry, type MenuRequest } from "./components/ContextMenu";
import { FileIcon } from "./components/FileIcon";
import { IconExport } from "./components/icons";
import { MapUriDialog, type UriMapping } from "./components/MapUriDialog";
import { Sidebar, type SidebarTarget } from "./components/Sidebar";
import { EditorController } from "./editor/controller";
import type { ConsoleEntry, ContextObject, Tab } from "./types";
import { baseName, dirName, loadOption, loadRecent, plural, pushRecent, saveOption, type RecentKind } from "./util";

const ALL_FILES = { name: "All files", extensions: ["*"] };
const DOCUMENT_FILTERS = [
  { name: "Documents", extensions: ["ocl", "oclinecore", "hutn"] },
  { name: "Complete OCL", extensions: ["ocl"] },
  { name: "OCLinEcore metamodels", extensions: ["oclinecore"] },
  { name: "HUTN models", extensions: ["hutn"] },
  ALL_FILES,
];
const SAVE_FILTERS: Record<Language, { name: string; extensions: string[] }[]> = {
  ocl: [{ name: "Complete OCL", extensions: ["ocl"] }, ALL_FILES],
  oclinecore: [{ name: "OCLinEcore metamodels", extensions: ["oclinecore"] }, ALL_FILES],
  hutn: [{ name: "HUTN models", extensions: ["hutn"] }, ALL_FILES],
  xmi: [{ name: "XMI models", extensions: ["xmi"] }, ALL_FILES],
};
const ECORE_FILTERS = [
  { name: "Metamodels", extensions: ["ecore", "oclinecore"] },
  { name: "Ecore metamodels", extensions: ["ecore"] },
  { name: "OCLinEcore metamodels", extensions: ["oclinecore"] },
  ALL_FILES,
];
const MODEL_FILTERS = [
  { name: "Models", extensions: ["xmi", "model", "hutn"] },
  { name: "XMI models", extensions: ["xmi", "model"] },
  { name: "HUTN models", extensions: ["hutn"] },
  ALL_FILES,
];

/** Replaces a path's extension. */
function withExtension(path: string, extension: string): string {
  const name = baseName(path);
  const dot = name.lastIndexOf(".");
  const stem = dot > 0 ? name.slice(0, dot) : name;
  const dir = dirName(path);
  return dir ? `${dir}/${stem}.${extension}` : `${stem}.${extension}`;
}

/** Where the cursor goes in a new document: the marker's position (the marker is removed). */
const CURSOR = "\u0000";

/** The initial text of a new document (a model of `pkg`); `CURSOR` marks where writing continues. */
function template(language: Language, n: number, pkg: PackageInfo | undefined): string {
  if (language === "oclinecore") {
    const name = n === 1 ? "example" : `example${n}`;
    return `-- A metamodel in OCLinEcore: Ecore classes with embedded OCL.
package ${name} : ${name} = 'http://www.example.org/${name}' {
  class Item {
    attribute name : String;
    attribute price : Integer;

    operation discounted(percent : Integer) : Integer {
      precondition ValidPercent: percent >= 0 and percent <= 100;
      body: price - (price * percent).div(100);
      postcondition NotMore: result <= price;
    }

    invariant PositivePrice('Item ' + name + ' has a negative price'): price >= 0;
  }
  ${CURSOR}
}
`;
  }
  if (language === "hutn") {
    if (!pkg) {
      return "// A model in HUTN. Open or write its metamodel first (an .oclinecore document or an\n// .ecore file); completion then proposes its classes and features.\n";
    }
    return `// A model in HUTN: objects are \`Class "identifier" { feature: value, ... }\`.
@Spec {
  metamodel "${pkg.name}" {
    nsUri: "${pkg.nsUri}"
  }
}

${pkg.name} {
  ${CURSOR}
}
`;
  }
  return "";
}

type Notice = { text: string; level: "info" | "error" };

function countObjects(objects: ObjectInfo[]): number {
  return objects.reduce((n, o) => n + 1 + countObjects(o.children), 0);
}

function containsObject(objects: ObjectInfo[], fragment: string): boolean {
  return objects.some((o) => o.fragment === fragment || containsObject(o.children, fragment));
}

function countPackages(packages: PackageInfo[]): number {
  return packages.reduce((n, p) => n + 1 + countPackages(p.subpackages), 0);
}

function containsPackage(p: PackageInfo, test: (p: PackageInfo) => boolean): boolean {
  return test(p) || p.subpackages.some((s) => containsPackage(s, test));
}

/** The root package containing the package `test` accepts. */
function rootPackage(packages: PackageInfo[], test: (p: PackageInfo) => boolean): PackageInfo | undefined {
  return packages.find((p) => containsPackage(p, test));
}

function shortName(qualified: string): string {
  return qualified.slice(qualified.lastIndexOf("::") + 2);
}

/** Splits a dialog result (string, string[] or null) into a list of paths. */
function paths(selection: string | string[] | null): string[] {
  if (!selection) return [];
  return Array.isArray(selection) ? selection : [selection];
}

export function App() {
  // ---------------------------------------------------------------- state
  const [tabs, setTabs] = useState<Tab[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [docErrors, setDocErrors] = useState<Map<string, number>>(() => new Map());
  const [packages, setPackages] = useState<PackageInfo[]>([]);
  const [models, setModels] = useState<ModelResourceInfo[]>([]);
  const [diagnostics, setDiagnostics] = useState<Diagnostic[]>([]);
  const [loadProblems, setLoadProblems] = useState<Diagnostic[]>([]);
  const [context, setContext] = useState<ContextObject | null>(null);
  const [report, setReport] = useState<ValidationReport | null>(null);
  const [running, setRunning] = useState(false);
  const [validationOptions, setValidationOptions] = useState<ValidationOptions>({
    includeSatisfied: false,
    includeEcore: true,
    contracts: true,
  });
  const [entries, setEntries] = useState<ConsoleEntry[]>([]);
  const [panelTab, setPanelTab] = useState<PanelTab>("problems");
  const [engine, setEngine] = useState(engineAvailable ? "Starting engine…" : "Engine unavailable");
  const [notice, setNotice] = useState<Notice | null>(null);
  const [recent, setRecent] = useState(() => ({
    ocl: loadRecent("ocl"),
    ecore: loadRecent("ecore"),
    model: loadRecent("model"),
  }));
  const [sidebarWidth, setSidebarWidth] = useState(300);
  const [panelHeight, setPanelHeight] = useState(250);
  const [uriMappings, setUriMappings] = useState<UriMapping[]>([]);
  const [showMapUri, setShowMapUri] = useState(false);
  const [menu, setMenu] = useState<MenuRequest | null>(null);
  /** Write the `.ecore` of an OCLinEcore document next to it whenever it is saved. */
  const [ecoreOnSave, setEcoreOnSave] = useState(() => loadOption("generateEcoreOnSave"));
  const ecoreOnSaveRef = useRef(ecoreOnSave);
  ecoreOnSaveRef.current = ecoreOnSave;

  const tabsRef = useRef(tabs);
  tabsRef.current = tabs;
  const packagesRef = useRef(packages);
  packagesRef.current = packages;
  const activeIdRef = useRef(activeId);
  activeIdRef.current = activeId;
  /** The model (HUTN) tab used last: where new objects go when no model tab is active. */
  const lastModelId = useRef<string | null>(null);
  const savedText = useRef(new Map<string, string>());
  const nextTabId = useRef(0);
  const nextEntryId = useRef(0);
  const untitledCounter = useRef(0);
  const noticeTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const refreshTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const refreshSeq = useRef(0);
  const lastJson = useRef({ packages: "", models: "" });
  const editorHost = useRef<HTMLDivElement>(null);

  // ---------------------------------------------------------------- notifications

  const addEntry = useCallback((entry: Omit<Extract<ConsoleEntry, { kind: "note" }>, "id">) => {
    setEntries((es) => {
      const last = es[es.length - 1];
      if (last?.kind === "note" && last.text === entry.text && !entry.diagnostics?.length) return es;
      return [...es, { ...entry, id: nextEntryId.current++ }];
    });
  }, []);

  const notify = useCallback(
    (text: string, level: "info" | "error" = "info") => {
      clearTimeout(noticeTimer.current);
      setNotice({ text, level });
      noticeTimer.current = setTimeout(() => setNotice(null), level === "error" ? 9000 : 5000);
      if (level === "error") addEntry({ kind: "note", level, text });
    },
    [addEntry],
  );

  // ---------------------------------------------------------------- workspace refresh

  const refreshWorkspace = useCallback(async () => {
    if (!engineAvailable) return;
    const seq = ++refreshSeq.current;
    try {
      const [pk, md, dg] = await Promise.all([rpc.packages(), rpc.models(), rpc.diagnostics()]);
      if (seq !== refreshSeq.current) return;
      const pkJson = JSON.stringify(pk);
      if (pkJson !== lastJson.current.packages) {
        lastJson.current.packages = pkJson;
        setPackages(pk);
      }
      const mdJson = JSON.stringify(md);
      if (mdJson !== lastJson.current.models) {
        lastJson.current.models = mdJson;
        setModels(md);
      }
      setDiagnostics(dg.diagnostics);
    } catch (e) {
      notify(errorMessage(e), "error");
    }
  }, [notify]);

  const scheduleRefresh = useCallback(() => {
    clearTimeout(refreshTimer.current);
    refreshTimer.current = setTimeout(() => void refreshWorkspace(), 120);
  }, [refreshWorkspace]);

  // ---------------------------------------------------------------- editor controller

  const handlers = useRef({ scheduleRefresh, notify });
  handlers.current = { scheduleRefresh, notify };

  const controller = useMemo(
    () =>
      new EditorController({
        onEdit: (id, text) => {
          const dirty = text !== savedText.current.get(id);
          setTabs((ts) =>
            ts.some((t) => t.id === id && t.dirty !== dirty) ? ts.map((t) => (t.id === id ? { ...t, dirty } : t)) : ts,
          );
        },
        onSynced: (id, diags) => {
          const errors = diags.filter((d) => d.severity === "error").length;
          setDocErrors((m) => {
            if ((m.get(id) ?? 0) === errors) return m;
            const next = new Map(m);
            next.set(id, errors);
            return next;
          });
          handlers.current.scheduleRefresh();
        },
        onError: (text) => handlers.current.notify(text, "error"),
      }),
    [],
  );

  useEffect(() => {
    const host = editorHost.current;
    if (host) controller.attach(host);
    return () => controller.detach();
  }, [controller]);

  // Startup: say hello to the engine.
  useEffect(() => {
    if (!engineAvailable) {
      addEntry({
        kind: "note",
        level: "error",
        text: "The engine runs in the desktop shell. Start the app with `npm run tauri dev` to load models and check documents.",
      });
      return;
    }
    rpc
      .initialize()
      .then((info) => setEngine(`${info.name} ${info.version}`))
      .catch((e) => {
        setEngine("Engine error");
        notify(errorMessage(e), "error");
      });
    void refreshWorkspace();
  }, [addEntry, notify, refreshWorkspace]);

  // Forget the context object when its model is gone.
  useEffect(() => {
    if (!context) return;
    const resource = models.find((m) => m.uri === context.resource);
    if (!resource || !containsObject(resource.roots, context.fragment)) setContext(null);
  }, [models, context]);

  const remember = useCallback((kind: RecentKind, path: string) => {
    const list = pushRecent(kind, path);
    setRecent((r) => ({ ...r, [kind]: list }));
  }, []);

  // ---------------------------------------------------------------- tabs

  const selectTab = useCallback(
    (id: string | null) => {
      controller.activate(id);
      setActiveId(id);
      if (id && controller.language(id) === "hutn") lastModelId.current = id;
      if (id) requestAnimationFrame(() => controller.focus());
    },
    [controller],
  );

  const addTab = useCallback(
    (name: string, path: string | null, marked: string, language: Language = languageOf(name), saved?: string) => {
      const id = `doc-${nextTabId.current++}`;
      const cursor = Math.max(0, marked.indexOf(CURSOR));
      const text = marked.replace(CURSOR, "");
      saved ??= text;
      savedText.current.set(id, saved);
      controller.open(id, name, text, language, cursor);
      setTabs((ts) => [...ts, { id, name, language, path, title: baseName(name), dirty: text !== saved }]);
      selectTab(id);
      return id;
    },
    [controller, selectTab],
  );

  /** A document name not used by another tab, based on `name`. */
  const freeName = useCallback((name: string) => {
    const taken = new Set(tabsRef.current.map((t) => t.name));
    if (!taken.has(name)) return name;
    const dot = name.lastIndexOf(".");
    const [stem, ext] = dot > 0 ? [name.slice(0, dot), name.slice(dot)] : [name, ""];
    for (let i = 2; ; i++) {
      const candidate = `${stem}-${i}${ext}`;
      if (!taken.has(candidate)) return candidate;
    }
  }, []);

  /** Opens a new document from a template (for models: of `pkg`, by default the first package). */
  const newDocument = useCallback(
    (language: Language = "ocl", pkg?: PackageInfo): string => {
      const taken = new Set(tabsRef.current.map((t) => t.name));
      let name: string;
      let n: number;
      do {
        n = ++untitledCounter.current;
        name = `untitled-${n}.${language}`;
      } while (taken.has(name));
      // An unsaved template is a change to save.
      return addTab(name, null, template(language, n, pkg ?? packagesRef.current[0]), language, "");
    },
    [addTab],
  );

  const openPath = useCallback(
    async (path: string) => {
      const existing = tabsRef.current.find((t) => t.path === path);
      if (existing) {
        selectTab(existing.id);
        return;
      }
      try {
        const text = await readTextFile(path);
        addTab(path, path, text);
        remember("ocl", path);
      } catch (e) {
        notify(errorMessage(e), "error");
      }
    },
    [addTab, notify, remember, selectTab],
  );

  const openDocuments = useCallback(async () => {
    try {
      const selection = await openDialog({ multiple: true, filters: DOCUMENT_FILTERS, title: "Open document" });
      // Metamodels first: models and constraints may need their classes.
      const selected = paths(selection).sort((a, b) => Number(languageOf(b) === "oclinecore") - Number(languageOf(a) === "oclinecore"));
      for (const p of selected) await openPath(p);
    } catch (e) {
      notify(errorMessage(e), "error");
    }
  }, [notify, openPath]);

  /** Converts an `.ecore` file to OCLinEcore, or an XMI model to HUTN, in a new tab. */
  const importAsText = useCallback(
    async (to: "oclinecore" | "hutn") => {
      try {
        const selection = await openDialog({
          multiple: false,
          filters:
            to === "oclinecore"
              ? [{ name: "Ecore metamodels", extensions: ["ecore"] }, ALL_FILES]
              : [{ name: "XMI models", extensions: ["xmi", "model"] }, ALL_FILES],
          title: to === "oclinecore" ? "Import .ecore metamodel as OCLinEcore" : "Import XMI model as HUTN",
        });
        for (const p of paths(selection)) {
          await controller.flush();
          const { text } = await rpc.convertFile(p, to);
          // Named next to the original, so relative imports keep working; saved on request.
          addTab(freeName(withExtension(p, to)), null, text, to, "");
          notify(`Converted ${baseName(p)} to ${to === "oclinecore" ? "OCLinEcore" : "HUTN"}`);
        }
      } catch (e) {
        notify(errorMessage(e), "error");
      }
    },
    [addTab, controller, freeName, notify],
  );

  /** Writes the `.ecore` of a saved OCLinEcore tab next to its file (`x.oclinecore` →
   * `x.ecore`). On save it is skipped while the document has errors; the outcome is returned
   * for the save notice instead of being shown. */
  const generateEcore = useCallback(
    async (tab: Tab, onSave = false): Promise<string | undefined> => {
      if (tab.language !== "oclinecore") return undefined;
      if (!tab.path) {
        notify("Save the metamodel first: its .ecore is written next to it.", "error");
        return undefined;
      }
      const target = withExtension(tab.path, "ecore");
      const file = baseName(target);
      try {
        await controller.flush();
        const { diagnostics } = await rpc.diagnostics(tab.name);
        const errors = diagnostics.filter((d) => d.severity === "error").length;
        if (errors > 0) {
          if (onSave) return `${file} not generated (${plural(errors, "error")})`;
          const go = await confirm(`${tab.title} has ${plural(errors, "error")}. Generate ${file} anyway?`, {
            title: "Generate .ecore",
            kind: "warning",
            okLabel: "Generate",
            cancelLabel: "Cancel",
          });
          if (!go) return undefined;
        }
        const { text } = await rpc.exportDocument(tab.name, target);
        await writeTextFile(target, text);
        if (onSave) return `generated ${file}`;
        notify(`Generated ${file}`);
      } catch (e) {
        const message = `Could not generate ${file}: ${errorMessage(e)}`;
        if (onSave) return message;
        notify(message, "error");
      }
      return undefined;
    },
    [controller, notify],
  );

  const writeTab = useCallback(
    async (tab: Tab, path: string) => {
      const text = controller.text(tab.id);
      await writeTextFile(path, text);
      savedText.current.set(tab.id, text);
      const language = languageOf(path) === "ocl" && !path.toLowerCase().endsWith(".ocl") ? tab.language : languageOf(path);
      controller.setLanguage(tab.id, language);
      if (tab.name !== path) await controller.rename(tab.id, path);
      // The text may have changed while writing; dirty compares against what was written.
      const dirty = controller.text(tab.id) !== text;
      setTabs((ts) =>
        ts.map((t) => (t.id === tab.id ? { ...t, name: path, path, language, title: baseName(path), dirty } : t)),
      );
      remember("ocl", path);
      const generated =
        language === "oclinecore" && ecoreOnSaveRef.current
          ? await generateEcore({ ...tab, name: path, path, language }, true)
          : undefined;
      notify(generated ? `Saved ${baseName(path)}, ${generated}` : `Saved ${baseName(path)}`);
    },
    [controller, generateEcore, notify, remember],
  );

  const saveAs = useCallback(
    async (id: string | null = activeIdRef.current): Promise<boolean> => {
      const tab = tabsRef.current.find((t) => t.id === id);
      if (!tab) return false;
      try {
        const path = await saveDialog({
          defaultPath: tab.path ?? (tab.name.includes("/") ? tab.name : tab.title),
          filters: SAVE_FILTERS[tab.language],
          title: "Save document",
        });
        if (!path) return false;
        if (tabsRef.current.some((t) => t.id !== tab.id && t.path === path)) {
          notify(`${baseName(path)} is already open in another tab. Close it first.`, "error");
          return false;
        }
        await writeTab(tab, path);
        return true;
      } catch (e) {
        notify(errorMessage(e), "error");
        return false;
      }
    },
    [notify, writeTab],
  );

  const saveTab = useCallback(
    async (id: string | null = activeIdRef.current): Promise<boolean> => {
      const tab = tabsRef.current.find((t) => t.id === id);
      if (!tab) return false;
      if (!tab.path) return saveAs(id);
      try {
        await writeTab(tab, tab.path);
        return true;
      } catch (e) {
        notify(errorMessage(e), "error");
        return false;
      }
    },
    [notify, saveAs, writeTab],
  );

  const closeTab = useCallback(
    async (id: string) => {
      const tab = tabsRef.current.find((t) => t.id === id);
      if (!tab) return;
      if (tab.dirty) {
        let choice: string;
        try {
          choice = await messageDialog(`Save changes to ${tab.title} before closing?`, {
            title: "Unsaved changes",
            kind: "warning",
            buttons: { yes: "Save", no: "Don't Save", cancel: "Cancel" },
          });
        } catch {
          choice = "Cancel";
        }
        if (choice === "Save" || choice === "Yes") {
          if (!(await saveTab(id))) return;
        } else if (choice !== "Don't Save" && choice !== "No") {
          return;
        }
      }
      const remaining = tabsRef.current.filter((t) => t.id !== id);
      if (activeIdRef.current === id) {
        const index = tabsRef.current.findIndex((t) => t.id === id);
        const neighbour = remaining[Math.min(index, remaining.length - 1)];
        selectTab(neighbour ? neighbour.id : null);
      }
      setTabs(remaining);
      setDocErrors((m) => {
        const next = new Map(m);
        next.delete(id);
        return next;
      });
      savedText.current.delete(id);
      await controller.close(id);
      scheduleRefresh();
    },
    [controller, saveTab, scheduleRefresh, selectTab],
  );

  /** Exports an OCLinEcore tab as an `.ecore` file, or a HUTN tab as an XMI file. */
  const exportTab = useCallback(
    async (id: string | null = activeIdRef.current) => {
      const tab = tabsRef.current.find((t) => t.id === id);
      if (!tab || (tab.language !== "oclinecore" && tab.language !== "hutn")) return;
      const extension = tab.language === "oclinecore" ? "ecore" : "xmi";
      try {
        const errors = docErrors.get(tab.id) ?? 0;
        if (errors > 0 && tab.language === "oclinecore") {
          const go = await confirm(`${tab.title} has ${plural(errors, "error")}. Export it anyway?`, {
            title: "Export",
            kind: "warning",
            okLabel: "Export",
            cancelLabel: "Cancel",
          });
          if (!go) return;
        }
        const target = await saveDialog({
          defaultPath: withExtension(tab.path ?? tab.name, extension),
          filters: [{ name: extension === "ecore" ? "Ecore metamodels" : "XMI models", extensions: [extension] }, ALL_FILES],
          title: extension === "ecore" ? "Export as .ecore" : "Export as XMI",
        });
        if (!target) return;
        await controller.flush();
        const { text } = await rpc.exportDocument(tab.name, target);
        await writeTextFile(target, text);
        notify(`Exported ${baseName(target)}`);
      } catch (e) {
        notify(errorMessage(e), "error");
      }
    },
    [controller, docErrors, notify],
  );

  /** Shows a document range: activates its tab and selects the range. */
  const jumpTo = useCallback(
    (documentName: string, range: Range) => {
      const tab = tabsRef.current.find((t) => t.name === documentName);
      if (!tab) {
        notify(`${baseName(documentName)} is not open.`);
        return;
      }
      selectTab(tab.id);
      controller.reveal(tab.id, range);
    },
    [controller, notify, selectTab],
  );

  const jumpToDiagnostic = useCallback(
    (d: Diagnostic) => {
      if (d.origin.kind === "document") jumpTo(d.origin.name, d.range);
    },
    [jumpTo],
  );

  const jumpToConstraint = useCallback(
    (c: ConstraintInfo) => {
      if (c.origin.kind === "document") jumpTo(c.origin.name, c.range);
      else notify(`${c.name} is defined in ${c.source}`);
    },
    [jumpTo, notify],
  );

  /** Shows the definition of an object of a HUTN document. */
  const jumpToObject = useCallback(
    (o: { resource: string; range?: Range }) => {
      if (o.range) jumpTo(o.resource, o.range);
    },
    [jumpTo],
  );

  // ---------------------------------------------------------------- creating objects

  /** The model tab new root objects go to: the active one, else the one used last. */
  const modelTarget = useCallback((): Tab | undefined => {
    const ids = [activeIdRef.current, lastModelId.current];
    return ids.map((id) => tabsRef.current.find((t) => t.id === id)).find((t) => t?.language === "hutn");
  }, []);

  /** Adds an object of class `cls` (qualified) to a model document: into `container`'s
   * feature, or as a root object of the tab `tabId` (`null`: of a new model). */
  const createObject = useCallback(
    async (cls: string, tabId: string | null, container?: { fragment: string; feature: string }) => {
      let id = tabId;
      if (id) {
        selectTab(id);
      } else {
        const ofClass = (p: PackageInfo) => p.classifiers.some((c) => c.qualifiedName === cls);
        id = newDocument("hutn", rootPackage(packagesRef.current, ofClass));
      }
      const name = controller.name(id);
      if (!name) return;
      const base = controller.text(id);
      try {
        const edit = await rpc.createObject(name, cls, container, base);
        if (!controller.applyEdit(id, base, edit)) notify(`${baseName(name)} changed meanwhile; try again.`, "error");
      } catch (e) {
        notify(errorMessage(e), "error");
      }
    },
    [controller, newDocument, notify, selectTab],
  );

  const closeMenu = useCallback(() => setMenu(null), []);

  /** The export menu of an OCLinEcore tab: generate the .ecore next to it, export it
   * elsewhere, or generate it on every save. */
  const openExportMenu = useCallback(
    (anchor: HTMLElement) => {
      const tab = tabsRef.current.find((t) => t.id === activeIdRef.current);
      if (!tab) return;
      const r = anchor.getBoundingClientRect();
      const ecore = baseName(withExtension(tab.path ?? tab.name, "ecore"));
      setMenu({
        x: r.left,
        y: r.bottom + 4,
        entries: [
          {
            kind: "item",
            label: `Generate ${ecore}`,
            hint: tab.path ? "next to the file" : "save first",
            icon: <FileIcon kind="ecore" />,
            disabled: !tab.path,
            onSelect: () => void generateEcore(tab),
          },
          { kind: "item", label: "Export as .ecore…", icon: <IconExport />, onSelect: () => void exportTab(tab.id) },
          { kind: "separator" },
          {
            kind: "item",
            label: "Generate .ecore on save",
            checked: ecoreOnSaveRef.current,
            onSelect: () => {
              const on = !ecoreOnSaveRef.current;
              setEcoreOnSave(on);
              saveOption("generateEcoreOnSave", on);
              notify(on ? "Saving a metamodel now writes its .ecore next to it" : "Saving no longer writes .ecore files");
            },
          },
        ],
      });
    },
    [exportTab, generateEcore, notify],
  );

  /** The context menu of a package, classifier or object in the sidebar. */
  const openSidebarMenu = useCallback(
    async (e: ReactMouseEvent<HTMLElement>, target: SidebarTarget) => {
      const { x, y } = menuPoint(e);
      const modelTab = modelTarget();
      const where = modelTab ? `in ${modelTab.title}` : "in a new model";
      const item = (label: string, onSelect: () => void, hint?: string, disabled?: boolean): MenuEntry => ({
        kind: "item",
        label,
        hint,
        disabled,
        onSelect,
      });
      const entries: MenuEntry[] = [];
      if (target.kind === "package") {
        const pkg = target.package;
        const root = rootPackage(packagesRef.current, (p) => p === pkg) ?? pkg;
        entries.push(item("New model", () => newDocument("hutn", root), ".hutn"));
        const classes = pkg.classifiers.filter((c) => c.kind === "class" && !c.isAbstract && !c.isInterface);
        if (classes.length) {
          entries.push({ kind: "separator" }, { kind: "header", label: `Create instance ${where}` });
          for (const c of classes) {
            entries.push(item(c.name, () => void createObject(c.qualifiedName, modelTab?.id ?? null)));
          }
        }
      } else if (target.kind === "classifier") {
        const c = target.classifier;
        if (c.kind !== "class") return;
        if (c.isAbstract || c.isInterface) {
          entries.push(item(`Create ${c.name}`, () => {}, c.isInterface ? "interface" : "abstract", true));
        } else {
          entries.push(item(`Create ${c.name}`, () => void createObject(c.qualifiedName, modelTab?.id ?? null), where));
          if (modelTab) {
            entries.push(item(`Create ${c.name} in a new model`, () => void createObject(c.qualifiedName, null)));
          }
        }
      } else {
        const { object: o, resource } = target;
        entries.push(
          item("Use as evaluation context", () =>
            setContext({ resource, fragment: o.fragment, label: o.label, class: o.class }),
          ),
        );
        if (o.range) entries.push(item("Show definition", () => jumpToObject({ resource, range: o.range })));
        const tab = tabsRef.current.find((t) => t.name === resource && t.language === "hutn");
        if (tab && o.range) {
          try {
            const options = await rpc.containmentOptions({ resource, fragment: o.fragment });
            for (const opt of options.filter((opt) => opt.classes.length)) {
              entries.push({ kind: "separator" }, { kind: "header", label: `Add to ${opt.feature}` });
              for (const cls of opt.classes) {
                entries.push(
                  item(
                    shortName(cls),
                    () => void createObject(cls, tab.id, { fragment: o.fragment, feature: opt.feature }),
                    opt.full ? "already set" : undefined,
                    opt.full,
                  ),
                );
              }
            }
          } catch (err) {
            notify(errorMessage(err), "error");
          }
        }
      }
      setMenu({ x, y, entries });
    },
    [createObject, jumpToObject, modelTarget, newDocument, notify],
  );

  // The browser's own menu (Reload, Inspect, …) is no use here; text fields keep theirs.
  useEffect(() => {
    const onContextMenu = (e: MouseEvent) => {
      if (!(e.target instanceof Element && e.target.closest("input, textarea, [contenteditable=true], .cm-editor"))) {
        e.preventDefault();
      }
    };
    window.addEventListener("contextmenu", onContextMenu);
    return () => window.removeEventListener("contextmenu", onContextMenu);
  }, []);

  // ---------------------------------------------------------------- loading

  const reportLoad = useCallback(
    (result: LoadResult) => {
      setLoadProblems((ps) => [...ps.filter((p) => !(p.origin.kind === "file" && p.origin.path === result.uri)), ...result.warnings]);
      const summary =
        result.objects !== undefined
          ? `Loaded ${baseName(result.uri)}: ${plural(result.objects, "object")}`
          : `Loaded ${baseName(result.uri)}: ${plural(result.packages?.length ?? 0, "package")}`;
      notify(summary);
      addEntry({
        kind: "note",
        level: "info",
        text: result.warnings.length ? `${summary}, ${plural(result.warnings.length, "warning")}` : summary,
        diagnostics: result.warnings,
      });
    },
    [addEntry, notify],
  );

  const loadMetamodelPath = useCallback(
    async (path: string) => {
      if (languageOf(path) === "oclinecore") {
        // A metamodel written in OCLinEcore is a document.
        await openPath(path);
        remember("ecore", path);
        return;
      }
      try {
        const result = await rpc.loadEcore(path);
        remember("ecore", path);
        reportLoad(result);
        // Documents that failed to resolve their types may check cleanly now.
        await controller.resyncAll();
      } catch (e) {
        notify(`Could not load ${baseName(path)}: ${errorMessage(e)}`, "error");
      }
      await refreshWorkspace();
    },
    [controller, notify, openPath, refreshWorkspace, remember, reportLoad],
  );

  const loadModelPath = useCallback(
    async (path: string) => {
      if (languageOf(path) === "hutn") {
        // A model written in HUTN is a document.
        await openPath(path);
        remember("model", path);
        return;
      }
      try {
        const result = await rpc.loadModel(path);
        remember("model", path);
        reportLoad(result);
      } catch (e) {
        notify(`Could not load ${baseName(path)}: ${errorMessage(e)}`, "error");
      }
      await refreshWorkspace();
    },
    [notify, openPath, refreshWorkspace, remember, reportLoad],
  );

  const loadMetamodel = useCallback(async () => {
    try {
      const selection = await openDialog({ multiple: true, filters: ECORE_FILTERS, title: "Load metamodel" });
      for (const p of paths(selection)) await loadMetamodelPath(p);
    } catch (e) {
      notify(errorMessage(e), "error");
    }
  }, [loadMetamodelPath, notify]);

  const loadModel = useCallback(async () => {
    try {
      const selection = await openDialog({ multiple: true, filters: MODEL_FILTERS, title: "Load model" });
      for (const p of paths(selection)) await loadModelPath(p);
    } catch (e) {
      notify(errorMessage(e), "error");
    }
  }, [loadModelPath, notify]);

  const mapUri = useCallback(
    async (m: UriMapping): Promise<boolean> => {
      try {
        await rpc.mapUri(m.from, m.to);
        setUriMappings((ms) => [...ms.filter((x) => x.from !== m.from), m]);
        notify(`Mapped ${m.from} to ${baseName(m.to)}`);
        addEntry({ kind: "note", level: "info", text: `Mapped ${m.from} to ${m.to}` });
        await refreshWorkspace();
        return true;
      } catch (e) {
        notify(`Could not map ${m.from}: ${errorMessage(e)}`, "error");
        return false;
      }
    },
    [addEntry, notify, refreshWorkspace],
  );

  const reload = useCallback(async () => {
    try {
      await controller.flush();
      const { problems } = await rpc.reload();
      setLoadProblems(problems);
      notify(problems.length ? `Reloaded with ${plural(problems.length, "problem")}` : "Reloaded metamodels and models");
      if (problems.length) addEntry({ kind: "note", level: "error", text: "Reload problems", diagnostics: problems });
      await controller.resyncAll();
    } catch (e) {
      notify(errorMessage(e), "error");
    }
    await refreshWorkspace();
  }, [addEntry, controller, notify, refreshWorkspace]);

  const resetWorkspace = useCallback(async () => {
    let ok = false;
    try {
      ok = await confirm(
        "Unload all metamodels and models and forget URI mappings? Open documents stay open and are checked again; their imports load again.",
        { title: "Reset workspace", kind: "warning", okLabel: "Reset", cancelLabel: "Cancel" },
      );
    } catch (e) {
      notify(errorMessage(e), "error");
    }
    if (!ok) return;
    try {
      await rpc.reset();
      setLoadProblems([]);
      setReport(null);
      setContext(null);
      setUriMappings([]);
      await controller.resyncAll();
      notify("Workspace reset");
    } catch (e) {
      notify(errorMessage(e), "error");
    }
    await refreshWorkspace();
  }, [controller, notify, refreshWorkspace]);

  // ---------------------------------------------------------------- check / run / evaluate

  const check = useCallback(async () => {
    await controller.resyncAll();
    await refreshWorkspace();
    setPanelTab("problems");
  }, [controller, refreshWorkspace]);

  const runValidation = useCallback(
    async (options: ValidationOptions = validationOptions) => {
      setRunning(true);
      setPanelTab("validation");
      try {
        await controller.flush();
        const r = await rpc.validate({
          ecore: options.includeEcore,
          includeSatisfied: options.includeSatisfied,
          contracts: options.contracts,
        });
        setReport(r);
      } catch (e) {
        notify(errorMessage(e), "error");
      } finally {
        setRunning(false);
      }
      scheduleRefresh();
    },
    [controller, notify, scheduleRefresh, validationOptions],
  );

  const changeValidationOptions = useCallback(
    (o: ValidationOptions) => {
      setValidationOptions(o);
      if (report) void runValidation(o);
    },
    [report, runValidation],
  );

  const evaluate = useCallback(
    async (expression: string) => {
      const id = nextEntryId.current++;
      const ctx = context;
      setEntries((es) => [...es, { kind: "evaluation", id, expression, context: ctx }]);
      const update = (patch: Partial<Extract<ConsoleEntry, { kind: "evaluation" }>>) =>
        setEntries((es) => es.map((e) => (e.id === id && e.kind === "evaluation" ? { ...e, ...patch } : e)));
      try {
        await controller.flush();
        const result = await rpc.evaluate(expression, ctx ? { resource: ctx.resource, fragment: ctx.fragment } : undefined);
        update({ result });
      } catch (e) {
        update({ error: errorMessage(e) });
      }
    },
    [context, controller],
  );

  const selectContext = useCallback(
    (c: ContextObject) => {
      setContext(c);
    },
    [],
  );

  const selectContextFromResult = useCallback(
    (o: ObjectSummary) => selectContext({ resource: o.resource, fragment: o.fragment, label: o.label, class: o.class }),
    [selectContext],
  );

  // ---------------------------------------------------------------- keyboard shortcuts

  const newLikeActive = useCallback(() => {
    const active = tabsRef.current.find((t) => t.id === activeIdRef.current);
    newDocument(active?.language ?? "ocl");
  }, [newDocument]);
  const shortcuts = useRef({ newLikeActive, openDocuments, saveTab, saveAs, runValidation, check });
  shortcuts.current = { newLikeActive, openDocuments, saveTab, saveAs, runValidation, check };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.altKey) return;
      const s = shortcuts.current;
      const key = e.key.toLowerCase();
      let action: (() => unknown) | null = null;
      if (key === "n" && !e.shiftKey) action = s.newLikeActive;
      else if (key === "o" && !e.shiftKey) action = s.openDocuments;
      else if (key === "s") action = e.shiftKey ? () => s.saveAs() : () => s.saveTab();
      else if (key === "enter") action = e.shiftKey ? s.check : () => s.runValidation();
      if (!action) return;
      e.preventDefault();
      e.stopPropagation();
      void action();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);

  // ---------------------------------------------------------------- splitters

  const dragSidebar = (e: ReactPointerEvent) => {
    const startX = e.clientX;
    const start = sidebarWidth;
    const move = (ev: PointerEvent) => setSidebarWidth(Math.min(600, Math.max(180, start + ev.clientX - startX)));
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      document.body.classList.remove("is-resizing");
    };
    document.body.classList.add("is-resizing");
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  const dragPanel = (e: ReactPointerEvent) => {
    const startY = e.clientY;
    const start = panelHeight;
    const move = (ev: PointerEvent) =>
      setPanelHeight(Math.min(window.innerHeight - 200, Math.max(110, start - (ev.clientY - startY))));
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      document.body.classList.remove("is-resizing", "is-resizing-row");
    };
    document.body.classList.add("is-resizing", "is-resizing-row");
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  // ---------------------------------------------------------------- derived

  const problems = useMemo(() => [...loadProblems, ...diagnostics], [loadProblems, diagnostics]);
  const counts = useMemo(
    () => ({
      errors: problems.filter((d) => d.severity === "error").length,
      warnings: problems.filter((d) => d.severity === "warning").length,
    }),
    [problems],
  );
  const metamodelFiles = useMemo(() => new Set(packages.map((p) => p.resource)).size, [packages]);
  const objectCount = useMemo(() => models.reduce((n, m) => n + countObjects(m.roots), 0), [models]);
  const activeTab = tabs.find((t) => t.id === activeId) ?? null;

  // ---------------------------------------------------------------- render

  return (
    <div
      className="app"
      style={{ gridTemplateColumns: `${sidebarWidth}px 1px minmax(0, 1fr)`, gridTemplateRows: `auto minmax(0, 1fr) auto` }}
    >
      <Toolbar
        canSave={activeTab !== null}
        canExport={activeTab?.language === "oclinecore" || activeTab?.language === "hutn"}
        exportLabel={activeTab?.language === "hutn" ? "Export as XMI…" : "Generate or export .ecore"}
        exportMenu={activeTab?.language === "oclinecore"}
        busy={running}
        onNew={newDocument}
        onImport={(to) => void importAsText(to)}
        onOpen={() => void openDocuments()}
        onSave={() => void saveTab()}
        onSaveAs={() => void saveAs()}
        onExport={(anchor) => (activeTab?.language === "oclinecore" ? openExportMenu(anchor) : void exportTab())}
        onLoadMetamodel={() => void loadMetamodel()}
        onLoadModel={() => void loadModel()}
        onReload={() => void reload()}
        onMapUri={() => setShowMapUri(true)}
        onCheck={() => void check()}
        onRun={() => void runValidation()}
        onReset={() => void resetWorkspace()}
      />
      <Sidebar
        packages={packages}
        models={models}
        context={context}
        onSelectContext={selectContext}
        onJumpObject={jumpToObject}
        onLoadMetamodel={() => void loadMetamodel()}
        onLoadModel={() => void loadModel()}
        onContextMenu={(e, target) => void openSidebarMenu(e, target)}
      />
      <div className="splitter splitter-col" onPointerDown={dragSidebar} role="separator" aria-orientation="vertical" />
      <main className="workbench" style={{ gridTemplateRows: `auto minmax(0, 1fr) 1px ${panelHeight}px` }}>
        <TabBar
          tabs={tabs}
          activeId={activeId}
          onSelect={selectTab}
          onClose={(id) => void closeTab(id)}
          onNew={newDocument}
          onImport={(to) => void importAsText(to)}
          errorCounts={docErrors}
        />
        <div className="editor-area">
          <div className="editor-host" ref={editorHost} hidden={tabs.length === 0} />
          {tabs.length === 0 && (
            <Welcome
              recent={recent}
              onNew={newDocument}
              onImport={(to) => void importAsText(to)}
              onOpen={() => void openDocuments()}
              onOpenPath={(p) => void openPath(p)}
              onLoadMetamodelPath={(p) => void loadMetamodelPath(p)}
              onLoadModelPath={(p) => void loadModelPath(p)}
            />
          )}
        </div>
        <div className="splitter splitter-row" onPointerDown={dragPanel} role="separator" aria-orientation="horizontal" />
        <BottomPanel tab={panelTab} onTab={setPanelTab} counts={counts} report={report}>
          {panelTab === "problems" && <ProblemsView problems={problems} onJump={jumpToDiagnostic} />}
          {panelTab === "validation" && (
            <ValidationView
              report={report}
              running={running}
              options={validationOptions}
              onOptions={changeValidationOptions}
              onRun={() => void runValidation()}
              onJumpConstraint={jumpToConstraint}
              onJumpDiagnostic={jumpToDiagnostic}
              onJumpObject={jumpToObject}
              onSelectObject={selectContextFromResult}
              context={context}
            />
          )}
          {panelTab === "console" && (
            <ConsoleView
              entries={entries}
              context={context}
              onEvaluate={(x) => void evaluate(x)}
              onClear={() => setEntries([])}
              onClearContext={() => setContext(null)}
              onJumpDiagnostic={jumpToDiagnostic}
              onJumpConstraint={jumpToConstraint}
            />
          )}
        </BottomPanel>
      </main>
      <StatusBar
        engine={engine}
        packageCount={countPackages(packages)}
        metamodelFiles={metamodelFiles}
        modelCount={models.length}
        objectCount={objectCount}
        context={context}
        errors={counts.errors}
        warnings={counts.warnings}
        report={report}
        message={notice}
        onProblems={() => setPanelTab("problems")}
        onValidation={() => setPanelTab("validation")}
        onConsole={() => setPanelTab("console")}
      />
      {menu && <ContextMenu request={menu} onClose={closeMenu} />}
      {showMapUri && <MapUriDialog mappings={uriMappings} onMap={mapUri} onClose={() => setShowMapUri(false)} />}
    </div>
  );
}
