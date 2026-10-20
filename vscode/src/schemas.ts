// XML Schemas for XMI models: the engine generates one per metamodel package; they are written
// to the extension's storage with an XML catalog that maps each nsURI to its schema, and the
// catalog is registered with the XML extension (redhat.vscode-xml), which then validates and
// completes XMI files as XML.

import * as vscode from "vscode";
import { errorMessage, type Engine, type SchemaFile } from "./engine";

/** The API of redhat.vscode-xml (see its `XMLExtensionApi`). */
interface XmlApi {
  addXMLCatalogs(catalogs: string[]): void;
  removeXMLCatalogs(catalogs: string[]): void;
}

export class SchemaRegistry {
  private written = "";
  private registered: string | undefined;

  constructor(
    private readonly engine: Engine,
    private readonly context: vscode.ExtensionContext,
    private readonly output: vscode.LogOutputChannel,
  ) {}

  /** Where the schemas go: a `file:` URI, which the XML language server can read (the storage
   * URI may have another scheme, e.g. `vscode-userdata:`). */
  private get folder(): vscode.Uri {
    return vscode.Uri.file(vscode.Uri.joinPath(this.context.globalStorageUri, "schemas").fsPath);
  }

  private async xmlApi(): Promise<XmlApi | undefined> {
    const xml = vscode.extensions.getExtension<XmlApi>("redhat.vscode-xml");
    if (!xml) return undefined;
    try {
      return xml.isActive ? xml.exports : await xml.activate();
    } catch {
      return undefined;
    }
  }

  /** Regenerates the schemas of the loaded metamodels (if they changed) and registers them. */
  async update(): Promise<void> {
    const enabled = vscode.workspace.getConfiguration("oxychloride").get<boolean>("xmlSchemas", true);
    if (!enabled) {
      await this.dispose();
      return;
    }
    let files: SchemaFile[];
    try {
      files = await this.engine.schemas();
    } catch (e) {
      this.output.warn(`Could not generate XML Schemas: ${errorMessage(e)}`);
      return;
    }
    const fingerprint = JSON.stringify(files);
    if (fingerprint === this.written) return;
    this.written = fingerprint;
    await vscode.workspace.fs.createDirectory(this.folder);
    for (const f of files) {
      await vscode.workspace.fs.writeFile(vscode.Uri.joinPath(this.folder, f.fileName), new TextEncoder().encode(f.text));
    }
    const catalog = vscode.Uri.joinPath(this.folder, "catalog.xml");
    await vscode.workspace.fs.writeFile(catalog, new TextEncoder().encode(catalogXml(files, this.folder)));
    this.output.info(`Wrote ${files.length} XML Schemas to ${this.folder.fsPath}`);
    const api = await this.xmlApi();
    if (!api) return;
    if (this.registered !== catalog.fsPath) {
      api.addXMLCatalogs([catalog.fsPath]);
      this.registered = catalog.fsPath;
    } else {
      // Make the XML extension read the changed schemas again.
      api.removeXMLCatalogs([catalog.fsPath]);
      api.addXMLCatalogs([catalog.fsPath]);
    }
  }

  /** Writes all schemas into `folder` (e.g. to use them outside VS Code). */
  async exportTo(folder: vscode.Uri): Promise<string[]> {
    const files = await this.engine.schemas();
    for (const f of files) {
      await vscode.workspace.fs.writeFile(vscode.Uri.joinPath(folder, f.fileName), new TextEncoder().encode(f.text));
    }
    return files.map((f) => f.fileName);
  }

  async dispose(): Promise<void> {
    if (!this.registered) return;
    (await this.xmlApi())?.removeXMLCatalogs([this.registered]);
    this.registered = undefined;
    this.written = "";
  }
}

function escape(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;");
}

/** An OASIS XML catalog mapping each namespace to its schema file. */
function catalogXml(files: SchemaFile[], folder: vscode.Uri): string {
  const entries = files
    .filter((f) => f.nsUri)
    .map((f) => `  <uri name="${escape(f.nsUri!)}" uri="${escape(vscode.Uri.joinPath(folder, f.fileName).toString())}"/>`);
  return `<?xml version="1.0" encoding="UTF-8"?>
<catalog xmlns="urn:oasis:names:tc:entity:xmlns:xml:catalog">
${entries.join("\n")}
</catalog>
`;
}
