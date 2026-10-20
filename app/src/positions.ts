// Conversion between the engine's positions (0-based line, column in Unicode scalar values)
// and CodeMirror offsets (UTF-16 code units from the start of the document).

import type { Position, Range } from "./api";

/** The subset of CodeMirror's `Text` used here (so tests can use plain objects too). */
export interface LineSource {
  readonly lines: number;
  readonly length: number;
  line(n: number): { from: number; to: number; text: string };
  lineAt(offset: number): { number: number; from: number; text: string };
}

/** UTF-16 index of the `column`-th scalar value in `text` (clamped to the line end). */
export function scalarColumnToUtf16(text: string, column: number): number {
  let index = 0;
  let scalars = 0;
  while (index < text.length && scalars < column) {
    const code = text.charCodeAt(index);
    const isHighSurrogate = code >= 0xd800 && code <= 0xdbff;
    const next = index + 1 < text.length ? text.charCodeAt(index + 1) : 0;
    index += isHighSurrogate && next >= 0xdc00 && next <= 0xdfff ? 2 : 1;
    scalars++;
  }
  return index;
}

/** Number of scalar values in `text` before UTF-16 index `index`. */
export function utf16ToScalarColumn(text: string, index: number): number {
  let i = 0;
  let scalars = 0;
  const end = Math.min(index, text.length);
  while (i < end) {
    const code = text.charCodeAt(i);
    const isHighSurrogate = code >= 0xd800 && code <= 0xdbff;
    const next = i + 1 < text.length ? text.charCodeAt(i + 1) : 0;
    i += isHighSurrogate && next >= 0xdc00 && next <= 0xdfff ? 2 : 1;
    scalars++;
  }
  return scalars;
}

/** Document offset of an engine position; out-of-range positions are clamped. */
export function positionToOffset(doc: LineSource, pos: Position): number {
  if (pos.line < 0) return 0;
  if (pos.line >= doc.lines) return doc.length;
  const line = doc.line(pos.line + 1);
  return line.from + scalarColumnToUtf16(line.text, Math.max(0, pos.column));
}

/** Engine position of a document offset. */
export function offsetToPosition(doc: LineSource, offset: number): Position {
  const clamped = Math.max(0, Math.min(offset, doc.length));
  const line = doc.lineAt(clamped);
  return { line: line.number - 1, column: utf16ToScalarColumn(line.text, clamped - line.from) };
}

/** Offsets of an engine range, normalised so that `from <= to`. */
export function rangeToOffsets(doc: LineSource, range: Range): { from: number; to: number } {
  const a = positionToOffset(doc, range.start);
  const b = positionToOffset(doc, range.end);
  return a <= b ? { from: a, to: b } : { from: b, to: a };
}
