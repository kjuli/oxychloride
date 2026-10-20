import { describe, expect, it } from "vitest";
import { Text } from "@codemirror/state";
import {
  offsetToPosition,
  positionToOffset,
  rangeToOffsets,
  scalarColumnToUtf16,
  utf16ToScalarColumn,
} from "./positions";

describe("column conversion", () => {
  it("is the identity for BMP text", () => {
    expect(scalarColumnToUtf16("abc", 2)).toBe(2);
    expect(utf16ToScalarColumn("abc", 2)).toBe(2);
  });

  it("counts astral characters as one scalar value", () => {
    const line = "a\u{1F600}b"; // a, emoji (2 code units), b
    expect(scalarColumnToUtf16(line, 1)).toBe(1);
    expect(scalarColumnToUtf16(line, 2)).toBe(3);
    expect(utf16ToScalarColumn(line, 3)).toBe(2);
    expect(utf16ToScalarColumn(line, 4)).toBe(3);
  });

  it("clamps past the line end", () => {
    expect(scalarColumnToUtf16("ab", 10)).toBe(2);
    expect(utf16ToScalarColumn("ab", 10)).toBe(2);
  });
});

describe("document positions", () => {
  const doc = Text.of(["context Book", "inv: name <> '\u{1F4D6}' and copies > 0", ""]);

  it("maps positions to offsets and back", () => {
    const pos = { line: 1, column: 16 }; // after the emoji string literal's closing quote
    const offset = positionToOffset(doc, pos);
    expect(doc.sliceString(offset - 2, offset)).toBe("\u{1F4D6}".slice(1) + "'");
    expect(offsetToPosition(doc, offset)).toEqual(pos);
  });

  it("handles line starts and the final empty line", () => {
    expect(positionToOffset(doc, { line: 0, column: 0 })).toBe(0);
    expect(positionToOffset(doc, { line: 1, column: 0 })).toBe(13);
    expect(positionToOffset(doc, { line: 2, column: 0 })).toBe(doc.length);
    expect(offsetToPosition(doc, doc.length)).toEqual({ line: 2, column: 0 });
  });

  it("clamps out-of-range lines", () => {
    expect(positionToOffset(doc, { line: 99, column: 3 })).toBe(doc.length);
  });

  it("normalises reversed ranges", () => {
    const r = rangeToOffsets(doc, { start: { line: 0, column: 12 }, end: { line: 0, column: 8 } });
    expect(r).toEqual({ from: 8, to: 12 });
    expect(doc.sliceString(r.from, r.to)).toBe("Book");
  });
});
