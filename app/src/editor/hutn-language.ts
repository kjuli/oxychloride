// HUTN (OMG Human-Usable Textual Notation): `Class "id" { feature: value, ... }`. Highlights
// class names, slot names (a name followed by `:`), identifiers, literals and `@Spec`.

import { StreamLanguage, syntaxHighlighting, type StringStream } from "@codemirror/language";

import { oclHighlightStyle } from "./ocl-language";

interface HutnState {
  inBlockComment: boolean;
}

function token(stream: StringStream, state: HutnState): string | null {
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
  if (stream.match("//")) {
    stream.skipToEnd();
    return "comment";
  }
  if (stream.match("/*")) {
    state.inBlockComment = true;
    return token(stream, state) ?? "comment";
  }
  const quote = stream.peek();
  if (quote === '"' || quote === "'") {
    stream.next();
    let escaped = false;
    let ch: string | void;
    while ((ch = stream.next()) != null) {
      if (ch === quote && !escaped) break;
      escaped = !escaped && ch === "\\";
    }
    return "string";
  }
  if (stream.match(/^-?\d+(\.\d+)?([eE][+-]?\d+)?/)) return "number";
  if (stream.match(/^@[A-Za-z]+/)) return "meta";
  if (stream.match("::")) return "punctuation";
  if (stream.match(/^[~]/)) return "operator";
  if (stream.match(/^[()[\]{}]/)) return "bracket";
  if (stream.match(/^[,;:]/)) return "punctuation";
  const word = stream.match(/^[A-Za-z_][A-Za-z0-9_]*/) as RegExpMatchArray | null;
  if (word) {
    const w = word[0];
    if (stream.match(/^\s*:(?!:)/, false)) return "propertyName";
    if (w === "true" || w === "false" || w === "null") return "atom";
    if (w === "metamodel") return "keyword";
    if (/^[A-Z][A-Z0-9_]+$/.test(w)) return "atom"; // enumeration literals are usually upper case
    if (/^[A-Z]/.test(w)) return "className";
    return "variableName";
  }
  stream.next();
  return null;
}

export const hutnLanguage = StreamLanguage.define<HutnState>({
  name: "hutn",
  startState: () => ({ inBlockComment: false }),
  copyState: (s) => ({ ...s }),
  token,
  languageData: {
    commentTokens: { line: "//", block: { open: "/*", close: "*/" } },
    closeBrackets: { brackets: ["(", "[", "{", '"'] },
    indentOnInput: /^\s*\}$/,
  },
});

export function hutn() {
  return [hutnLanguage, syntaxHighlighting(oclHighlightStyle)];
}
