// Document names as the language server gives them to the engine, and engine positions
// (columns in Unicode scalar values) as VS Code positions (columns in UTF-16 code units).

import * as vscode from "vscode";
import type { DocumentEdit, Position, Range } from "./engine";

/** The engine's name of a document: its path, or (unsaved documents) its URI. Mirrors
 * `document_name` in crates/oxy-lsp. */
export function documentName(uri: vscode.Uri): string {
  if (uri.scheme !== "file") return uri.toString();
  // `/c:/dir/x.ocl` is a Windows path.
  return /^\/[A-Za-z]:/.test(uri.path) ? uri.path.slice(1) : uri.path;
}

/** The URI of a document (or file) the engine names `name`. */
export function uriOf(name: string): vscode.Uri {
  const open = vscode.workspace.textDocuments.find((d) => documentName(d.uri) === name);
  if (open) return open.uri;
  return /^([A-Za-z]:)?[\\/]/.test(name) ? vscode.Uri.file(name) : vscode.Uri.parse(name);
}

/** The open document the engine names `name`, if any. */
export function openDocument(name: string): vscode.TextDocument | undefined {
  return vscode.workspace.textDocuments.find((d) => documentName(d.uri) === name);
}

function utf16Column(line: string, column: number): number {
  let index = 0;
  for (let scalars = 0; index < line.length && scalars < column; scalars++) {
    const code = line.charCodeAt(index);
    index += code >= 0xd800 && code <= 0xdbff && index + 1 < line.length ? 2 : 1;
  }
  return index;
}

export function toVsPosition(doc: vscode.TextDocument, p: Position): vscode.Position {
  if (p.line >= doc.lineCount) return doc.lineAt(doc.lineCount - 1).range.end;
  return new vscode.Position(p.line, utf16Column(doc.lineAt(p.line).text, p.column));
}

export function toVsRange(doc: vscode.TextDocument, r: Range): vscode.Range {
  return new vscode.Range(toVsPosition(doc, r.start), toVsPosition(doc, r.end));
}

/** Opens the document the engine names `name` and selects `range` in it. */
export async function reveal(name: string, range?: Range): Promise<void> {
  const doc = await vscode.workspace.openTextDocument(uriOf(name));
  const selection = range ? toVsRange(doc, range) : undefined;
  await vscode.window.showTextDocument(doc, { selection, preview: false });
}

/** Applies an engine edit computed for the text `base` of `doc` and moves the cursor to
 * `edit.cursor`. Returns false if the document changed meanwhile. */
export async function applyEdit(doc: vscode.TextDocument, base: string, edit: DocumentEdit): Promise<boolean> {
  if (doc.getText() !== base) return false;
  const workspaceEdit = new vscode.WorkspaceEdit();
  for (const e of edit.edits) workspaceEdit.replace(doc.uri, toVsRange(doc, e.range), e.text);
  if (!(await vscode.workspace.applyEdit(workspaceEdit))) return false;
  const editor = await vscode.window.showTextDocument(doc, { preview: false });
  const cursor = toVsPosition(doc, edit.cursor);
  editor.selection = new vscode.Selection(cursor, cursor);
  editor.revealRange(new vscode.Range(cursor, cursor), vscode.TextEditorRevealType.InCenterIfOutsideViewport);
  return true;
}
