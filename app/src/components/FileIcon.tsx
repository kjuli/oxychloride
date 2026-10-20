// The icons of the document kinds (assets/file-icons, shared with the VS Code extension), in
// their light or dark variant.

import ecoreDark from "../../../assets/file-icons/ecore-dark.svg";
import ecore from "../../../assets/file-icons/ecore.svg";
import hutnDark from "../../../assets/file-icons/hutn-dark.svg";
import hutn from "../../../assets/file-icons/hutn.svg";
import oclDark from "../../../assets/file-icons/ocl-dark.svg";
import ocl from "../../../assets/file-icons/ocl.svg";
import oclinecoreDark from "../../../assets/file-icons/oclinecore-dark.svg";
import oclinecore from "../../../assets/file-icons/oclinecore.svg";
import xmiDark from "../../../assets/file-icons/xmi-dark.svg";
import xmi from "../../../assets/file-icons/xmi.svg";

export type FileKind = "ocl" | "oclinecore" | "hutn" | "ecore" | "xmi";

const ICONS: Record<FileKind, { light: string; dark: string }> = {
  ocl: { light: ocl, dark: oclDark },
  oclinecore: { light: oclinecore, dark: oclinecoreDark },
  hutn: { light: hutn, dark: hutnDark },
  ecore: { light: ecore, dark: ecoreDark },
  xmi: { light: xmi, dark: xmiDark },
};

/** The kind of a file, by its extension (`.model` files are XMI too). */
export function fileKind(path: string): FileKind | null {
  const ext = path.slice(path.lastIndexOf(".") + 1).toLowerCase();
  if (ext === "model") return "xmi";
  return ext in ICONS ? (ext as FileKind) : null;
}

export function FileIcon({ kind, title }: { kind: FileKind; title?: string }) {
  const icon = ICONS[kind];
  return (
    <picture className="file-icon" title={title}>
      <source srcSet={icon.dark} media="(prefers-color-scheme: dark)" />
      <img src={icon.light} width={16} height={16} alt={title ?? ""} />
    </picture>
  );
}
