/** Last path segment of a file path or URI. */
export function baseName(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  const i = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  return i >= 0 ? trimmed.slice(i + 1) : trimmed;
}

/** Directory part of a path (empty if none). */
export function dirName(path: string): string {
  const i = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return i > 0 ? path.slice(0, i) : "";
}

export function plural(n: number, singular: string, pluralForm = `${singular}s`): string {
  return `${n} ${n === 1 ? singular : pluralForm}`;
}

// ---------------------------------------------------------------- options

/** An on/off option remembered across sessions (off if storage is unavailable). */
export function loadOption(name: string): boolean {
  try {
    return localStorage.getItem(`oxychloride.option.${name}`) === "true";
  } catch {
    return false;
  }
}

export function saveOption(name: string, value: boolean): void {
  try {
    localStorage.setItem(`oxychloride.option.${name}`, String(value));
  } catch {
    // Storage may be unavailable; the option then lasts for this session.
  }
}

// ---------------------------------------------------------------- recent files

export type RecentKind = "ocl" | "ecore" | "model";
const RECENT_LIMIT = 8;

export function loadRecent(kind: RecentKind): string[] {
  try {
    const raw = localStorage.getItem(`oxychloride.recent.${kind}`);
    const parsed: unknown = raw ? JSON.parse(raw) : [];
    return Array.isArray(parsed) ? parsed.filter((p): p is string => typeof p === "string") : [];
  } catch {
    return [];
  }
}

export function pushRecent(kind: RecentKind, path: string): string[] {
  const next = [path, ...loadRecent(kind).filter((p) => p !== path)].slice(0, RECENT_LIMIT);
  try {
    localStorage.setItem(`oxychloride.recent.${kind}`, JSON.stringify(next));
  } catch {
    // Storage may be unavailable; recent files are a convenience only.
  }
  return next;
}
