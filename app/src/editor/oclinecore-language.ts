// OCLinEcore: Ecore metamodels as text with embedded OCL. The OCL tokenizer handles both the
// structure (with these extra keywords and qualifiers) and the embedded expressions.

import { StreamLanguage, syntaxHighlighting } from "@codemirror/language";

import { oclHighlightStyle, oclTokenizer, type OclState } from "./ocl-language";

const KEYWORDS = new Set([
  // structure
  "module", "import", "library", "package", "class", "abstract", "interface", "extends", "datatype",
  "primitive", "enum", "literal", "attribute", "property", "operation", "throws", "annotation",
  "documentation",
  // constraints
  "invariant", "inv", "callable", "precondition", "pre", "postcondition", "post", "body", "derivation",
  "derive", "initial", "init",
  // OCL
  "let", "in", "if", "then", "else", "elseif", "endif", "and", "or", "xor", "not", "implies",
]);

const MODIFIERS = new Set([
  "composes", "derived", "id", "ordered", "readonly", "resolve", "transient", "unique", "unsettable",
  "volatile", "serializable", "static", "definition", "key",
]);

export const oclinecoreLanguage = StreamLanguage.define<OclState>({
  name: "oclinecore",
  startState: () => ({ inBlockComment: false, afterNavigation: false }),
  copyState: (s) => ({ ...s }),
  token: oclTokenizer({ keywords: KEYWORDS, modifiers: MODIFIERS }),
  languageData: {
    commentTokens: { line: "--", block: { open: "/*", close: "*/" } },
    closeBrackets: { brackets: ["(", "[", "{", "'"] },
    indentOnInput: /^\s*\}$/,
  },
});

export function oclinecore() {
  return [oclinecoreLanguage, syntaxHighlighting(oclHighlightStyle)];
}
