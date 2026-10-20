// Drives the extension in a real VS Code: live diagnostics for HUTN and XMI models, the views,
// creating objects, the OCL console and validation.

import * as assert from "node:assert";
import * as fs from "node:fs";
import * as path from "node:path";
import * as vscode from "vscode";

/** The parts of the extension's API the tests use. */
interface Api {
  ready: Promise<void>;
  engine: { call<T>(method: string, params?: object | null): Promise<T> };
  console: {
    evaluate(expression: string): Promise<{ display: string; type: string; contracts?: unknown[] } | undefined>;
    setContext(c: { resource: string; fragment: string; label: string; class: string } | undefined): void;
  };
  creator: {
    createInstance(cls: unknown, inNew?: boolean): Promise<boolean>;
    addChild(node: unknown, choice?: { feature: string; cls: string }): Promise<boolean>;
  };
  metamodels: { packages: { name: string; classifiers: { name: string; qualifiedName: string }[] }[] };
  models: { models: { uri: string; roots: { fragment: string; label: string; children: unknown[] }[] }[] };
  validation: { report?: { summary: { violated: number } } };
}

const folder = () => vscode.workspace.workspaceFolders![0].uri.fsPath;

async function waitFor<T>(what: string, check: () => T | undefined | false, timeout = 30000): Promise<T> {
  const start = Date.now();
  for (;;) {
    const value = check();
    if (value) return value;
    if (Date.now() - start > timeout) throw new Error(`timed out waiting for ${what}`);
    await new Promise((r) => setTimeout(r, 100));
  }
}

const errors = (uri: vscode.Uri) =>
  vscode.languages.getDiagnostics(uri).filter((d) => d.severity === vscode.DiagnosticSeverity.Error);

describe("Oxychloride", () => {
  let api: Api;

  before(async () => {
    const ext = vscode.extensions.getExtension<Api>("palladio.oxychloride")!;
    api = await ext.activate();
    await api.ready;
  });

  it("checks a HUTN model against the constraints of a metamodel that is not open", async () => {
    const uri = vscode.Uri.file(path.join(folder(), "university/university.hutn"));
    await vscode.window.showTextDocument(uri);
    const diagnostics = await waitFor("5 violations", () => errors(uri).length === 5 && errors(uri));
    assert.ok(diagnostics.some((d) => d.message.includes("CreditLimit")), diagnostics.map((d) => d.message).join("\n"));
    assert.ok(diagnostics.some((d) => d.message.includes("postcondition 'FreeSeat'")));
  });

  it("re-checks while typing", async () => {
    const editor = vscode.window.activeTextEditor!;
    const doc = editor.document;
    const thesis = doc.getText().indexOf('          Enrolment { course: Course "thesis" }');
    const line = doc.positionAt(thesis).line;
    // Carla drops the thesis: she is within the credit limit then.
    await editor.edit((e) => {
      e.delete(doc.lineAt(line).rangeIncludingLineBreak);
      const previous = doc.lineAt(line - 1);
      e.delete(new vscode.Range(previous.range.end.translate(0, -1), previous.range.end));
    });
    await waitFor("CreditLimit fixed", () => !errors(doc.uri).some((d) => d.message.includes("CreditLimit")));
    await vscode.commands.executeCommand("undo");
    await waitFor("CreditLimit back", () => errors(doc.uri).some((d) => d.message.includes("CreditLimit")));
  });

  it("shows metamodels and models", async () => {
    await waitFor("packages", () => api.metamodels.packages.some((p) => p.name === "university"));
    await waitFor("the model", () => api.models.models.some((m) => m.uri.endsWith("university.hutn")));
  });

  it("creates instances and children", async () => {
    const pkg = api.metamodels.packages.find((p) => p.name === "university")!;
    const course = pkg.classifiers.find((c) => c.name === "Course")!;
    const doc = vscode.window.activeTextEditor!.document;
    assert.ok(await api.creator.createInstance(course));
    assert.ok(doc.getText().includes("\n  Course {  }\n}"), doc.getText());
    const cursor = vscode.window.activeTextEditor!.selection.active;
    assert.strictEqual(doc.lineAt(cursor.line).text.slice(cursor.character - 2, cursor.character + 2), "{  }");

    const model = await waitFor("the model", () => api.models.models.find((m) => m.uri.endsWith("university.hutn")));
    const tum = model.roots[0];
    const node = { kind: "object", object: tum, resource: model.uri };
    assert.ok(await api.creator.addChild(node, { feature: "students", cls: "university::Student" }));
    assert.ok(doc.getText().includes("      },\n      Student {  }\n  }"), doc.getText());
    await vscode.commands.executeCommand("workbench.action.files.revert");

    // In a new, unsaved model.
    assert.ok(await api.creator.createInstance(course, true));
    const untitled = vscode.window.activeTextEditor!.document;
    assert.strictEqual(untitled.uri.scheme, "untitled");
    assert.ok(untitled.getText().includes("university {\n  Course {  }\n}"), untitled.getText());
    // Its objects are checked like those of saved models.
    await waitFor("required features", () =>
      vscode.languages.getDiagnostics(untitled.uri).some((d) => d.message.includes("required feature 'code'")),
    );
    await vscode.commands.executeCommand("workbench.action.revertAndCloseActiveEditor");
  });

  it("evaluates OCL in the console", async () => {
    const all = await api.console.evaluate("Student.allInstances()->collect(name)->sortedBy(n | n)");
    assert.strictEqual(all?.display, "Sequence{'Ada', 'Brian', 'Carla'}");
    const model = api.models.models.find((m) => m.uri.endsWith("university.hutn"))!;
    api.console.setContext({ resource: model.uri, fragment: "ada", label: "Ada", class: "Student" });
    const self = await api.console.evaluate("self.mayEnrol(Course.allInstances()->any(code = 'IN0001'))");
    assert.strictEqual(self?.type, "Boolean");
    assert.ok(self?.contracts?.length, "the violated precondition of the call");
    api.console.setContext(undefined);
  });

  it("validates", async () => {
    await vscode.commands.executeCommand("oxychloride.validate");
    assert.strictEqual(api.validation.report?.summary.violated, 5);
  });

  it("checks XMI models live", async () => {
    const file = path.join(folder(), "university/tum.xmi");
    fs.writeFileSync(
      file,
      `<?xml version="1.0" encoding="UTF-8"?>
<uni:University xmi:version="2.0" xmlns:xmi="http://www.omg.org/XMI" xmlns:uni="http://www.example.org/university/1.0" name="TUM">
  <courses code="IN1" title="Programming" kind="LECTURE" credits="8" capacity="0"/>
  <students name="Dora" semester="1">
    <enrolments course="//@courses.0"/>
  </students>
</uni:University>
`,
    );
    const uri = vscode.Uri.file(file);
    const doc = await vscode.workspace.openTextDocument(uri);
    await vscode.window.showTextDocument(doc);
    assert.strictEqual(doc.languageId, "xml");
    // Dora is enrolled in a course without capacity.
    const fromOcl = () => errors(uri).filter((d) => d.source === "ocl");
    const diagnostics = await waitFor("a violation", () => fromOcl().length > 0 && fromOcl());
    assert.ok(diagnostics.some((d) => d.message.includes("NotOverbooked")), diagnostics.map((d) => d.message).join("\n"));
    const typo = doc.getText().indexOf('semester="1"');
    const edit = new vscode.WorkspaceEdit();
    edit.replace(uri, new vscode.Range(doc.positionAt(typo), doc.positionAt(typo + 8)), "semestre");
    await vscode.workspace.applyEdit(edit);
    await waitFor("unknown feature", () => errors(uri).some((d) => d.message.includes("no feature 'semestre'")));
  });

  it("validates XMI models with the generated XML Schemas", async function () {
    const xml = vscode.extensions.getExtension("redhat.vscode-xml");
    if (!xml) this.skip();
    await xml!.activate();
    // The OCL engine and the XML extension both report the unknown attribute of the last test.
    const uri = vscode.Uri.file(path.join(folder(), "university/tum.xmi"));
    const fromXml = (d: vscode.Diagnostic) => d.source !== "ocl";
    const diagnostics = await waitFor(
      "a schema violation",
      () => vscode.languages.getDiagnostics(uri).some(fromXml) && vscode.languages.getDiagnostics(uri).filter(fromXml),
      90000,
    );
    const messages = diagnostics.map((d) => `${d.source}: ${d.message}`).join("\n");
    assert.ok(messages.includes("semestre"), messages);
    // Fixed, the document is valid XML for its schema: no complaints about the namespace either.
    const doc = await vscode.workspace.openTextDocument(uri);
    const edit = new vscode.WorkspaceEdit();
    const typo = doc.getText().indexOf("semestre");
    edit.replace(uri, new vscode.Range(doc.positionAt(typo), doc.positionAt(typo + 8)), "semester");
    await vscode.workspace.applyEdit(edit);
    await waitFor("no schema problems", () => !vscode.languages.getDiagnostics(uri).some(fromXml), 30000);
  });

  it("generates the .ecore of an OCLinEcore metamodel", async () => {
    const source = vscode.Uri.file(path.join(folder(), "university/university.oclinecore"));
    const ecore = path.join(folder(), "university/university.ecore");
    const target = await vscode.commands.executeCommand<vscode.Uri>("oxychloride.generateEcore", source);
    assert.strictEqual(target?.fsPath, vscode.Uri.file(ecore).fsPath);
    const xml = fs.readFileSync(ecore, "utf8");
    assert.ok(xml.includes('nsURI="http://www.example.org/university/1.0"') && xml.includes('name="Course"'), xml);
  });

  it("generates the .ecore on save, unless the metamodel has errors", async () => {
    const config = vscode.workspace.getConfiguration("oxychloride");
    await config.update("generateEcoreOnSave", true, vscode.ConfigurationTarget.Workspace);
    try {
      const ecore = path.join(folder(), "university/university.ecore");
      fs.rmSync(ecore, { force: true });
      const doc = await vscode.workspace.openTextDocument(path.join(folder(), "university/university.oclinecore"));
      const editor = await vscode.window.showTextDocument(doc);
      const end = doc.lineAt(doc.lineCount - 1).range.end;
      await editor.edit((e) => e.insert(end, "\n-- generated on save\n"));
      await doc.save();
      await waitFor("the generated .ecore", () => fs.existsSync(ecore));
      assert.ok(fs.readFileSync(ecore, "utf8").includes('name="Student"'));

      // With an error, the .ecore is left alone.
      fs.rmSync(ecore);
      const course = doc.getText().indexOf("class Course {");
      await editor.edit((e) => e.replace(new vscode.Range(doc.positionAt(course), doc.positionAt(course + 14)), "class Course extends Nowhere {"));
      await doc.save();
      await new Promise((r) => setTimeout(r, 1500));
      assert.ok(!fs.existsSync(ecore), "no .ecore from a metamodel with errors");
      await vscode.commands.executeCommand("undo");
      await vscode.commands.executeCommand("undo");
      await doc.save();
      await waitFor("the generated .ecore again", () => fs.existsSync(ecore));
    } finally {
      await config.update("generateEcoreOnSave", undefined, vscode.ConfigurationTarget.Workspace);
    }
  });

  it("has icons for its languages", () => {
    const ext = vscode.extensions.getExtension("palladio.oxychloride")!;
    const languages: { id: string; icon?: { light: string; dark: string } }[] = ext.packageJSON.contributes.languages;
    for (const id of ["ocl", "oclinecore", "hutn"]) {
      const icon = languages.find((l) => l.id === id)?.icon;
      assert.ok(icon, `no icon for ${id}`);
      for (const file of [icon.light, icon.dark]) {
        const svg = fs.readFileSync(path.join(ext.extensionPath, file), "utf8");
        assert.ok(svg.startsWith("<svg"), file);
      }
    }
  });
});
