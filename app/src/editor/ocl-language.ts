// A small stream tokenizer for OCL: enough for highlighting, comment toggling and bracket
// matching. Semantic checking comes from the engine. `oclTokenizer` is shared with the
// OCLinEcore language, which embeds OCL.

import { HighlightStyle, StreamLanguage, syntaxHighlighting, type StringStream } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";

const KEYWORDS = new Set([
  "context", "inv", "def", "init", "derive", "pre", "post", "body", "package", "endpackage",
  "import", "let", "in", "if", "then", "else", "endif", "and", "or", "xor", "not", "implies",
  "static", "attr", "oper",
]);

const ATOMS = new Set(["self", "result", "true", "false", "null", "invalid"]);

const TYPES = new Set([
  "Boolean", "Integer", "Real", "String", "UnlimitedNatural", "OclAny", "OclVoid", "OclInvalid",
  "OclType", "OclElement", "Set", "Bag", "Sequence", "OrderedSet", "Collection", "Tuple", "Map",
]);

const ITERATORS = new Set([
  "forAll", "exists", "select", "reject", "collect", "collectNested", "iterate", "any", "one",
  "isUnique", "sortedBy", "closure", "selectByKind", "selectByType",
]);

export interface OclState {
  inBlockComment: boolean;
  /** The previous significant token was `.` or `->`, so an identifier is a member name. */
  afterNavigation: boolean;
}

/** Words of a host language around OCL: keywords and (optionally) qualifiers. */
export interface OclDialect {
  keywords: Set<string>;
  modifiers?: Set<string>;
}

export function oclTokenizer(dialect: OclDialect) {
  return (stream: StringStream, state: OclState) => token(stream, state, dialect);
}

function token(stream: StringStream, state: OclState, dialect: OclDialect): string | null {
  if (state.inBlockComment) {
    if (stream.skipTo("*/")) {
      stream.match("*/");
      state.inBlockComment = false;
    } else {
      stream.skipToEnd();
    }
    return "comment";
  }
  if (stream.eatSpace()) return null;

  if (stream.match("--")) {
    stream.skipToEnd();
    return "comment";
  }
  if (stream.match("/*")) {
    state.inBlockComment = true;
    return token(stream, state, dialect) ?? "comment";
  }

  const navigation = state.afterNavigation;
  state.afterNavigation = false;

  if (stream.peek() === "'") {
    stream.next();
    let escaped = false;
    let ch: string | void;
    while ((ch = stream.next()) != null) {
      if (ch === "'" && !escaped) break;
      escaped = !escaped && ch === "\\";
    }
    return "string";
  }
  if (stream.match(/^\d+(\.\d+(?!\.))?([eE][+-]?\d+)?/)) return "number";
  if (stream.match("->") || stream.match(".")) {
    state.afterNavigation = true;
    return "operator";
  }
  if (stream.match("::")) return "punctuation";
  if (stream.match(/^(<>|<=|>=|[=<>+\-*/|@^#!])/)) return "operator";
  if (stream.match(/^[()[\]{}]/)) return "bracket";
  if (stream.match(/^[,;:]/)) return "punctuation";

  const word = stream.match(/^[A-Za-z_][A-Za-z0-9_]*/) as RegExpMatchArray | null;
  if (word) {
    const w = word[0];
    if (navigation) return ITERATORS.has(w) ? "macroName" : "propertyName";
    if (dialect.keywords.has(w)) return "keyword";
    if (dialect.modifiers?.has(w)) return "modifier";
    if (ATOMS.has(w)) return "atom";
    if (TYPES.has(w)) return "typeName";
    if (/^[A-Z]/.test(w)) return "className";
    return "variableName";
  }
  stream.next();
  return null;
}

export const oclLanguage = StreamLanguage.define<OclState>({
  name: "ocl",
  startState: () => ({ inBlockComment: false, afterNavigation: false }),
  copyState: (s) => ({ ...s }),
  token: oclTokenizer({ keywords: KEYWORDS }),
  languageData: {
    commentTokens: { line: "--", block: { open: "/*", close: "*/" } },
    closeBrackets: { brackets: ["(", "[", "{", "'"] },
  },
});

// Colours come from CSS custom properties so the editor follows the light/dark theme.
export const oclHighlightStyle = HighlightStyle.define([
  { tag: t.keyword, color: "var(--syn-keyword)", fontWeight: "600" },
  { tag: t.comment, color: "var(--syn-comment)", fontStyle: "italic" },
  { tag: t.string, color: "var(--syn-string)" },
  { tag: t.number, color: "var(--syn-number)" },
  { tag: t.atom, color: "var(--syn-atom)" },
  { tag: [t.typeName, t.className], color: "var(--syn-type)" },
  { tag: t.propertyName, color: "var(--syn-property)" },
  { tag: t.macroName, color: "var(--syn-iterator)" },
  { tag: t.modifier, color: "var(--syn-iterator)", fontStyle: "italic" },
  { tag: t.meta, color: "var(--syn-keyword)", fontWeight: "600" },
  { tag: t.operator, color: "var(--syn-operator)" },
  { tag: [t.bracket, t.punctuation], color: "var(--syn-punct)" },
]);

export function ocl() {
  return [oclLanguage, syntaxHighlighting(oclHighlightStyle)];
}
