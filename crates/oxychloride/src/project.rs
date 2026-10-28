//! Modeling projects: a folder with a manifest, `oxychloride.json`, that holds metamodels,
//! constraints, models and transformations (see docs/PROJECTS.md).
//!
//! The manifest names the project, the folders new documents go into, the project's package
//! registry (URIs → metamodel files, relative to the project) and the defaults of code
//! generation. Every document below the project folder belongs to it, wherever it is (folders
//! are where new documents go, not a restriction), except those of nested projects, hidden
//! folders, build output and the folder of generated code.
//!
//! New projects and new documents are written from templates here, so that every front end
//! (`oxy new`, the VS Code command, other clients of `newProject`) creates the same; a new
//! project is checked before it is returned, so its documents are free of errors.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::protocol::*;
use crate::workspace::{Error, Workspace};

/// The file name of a project's manifest.
pub const MANIFEST: &str = "oxychloride.json";

/// Folders below a project that never hold its documents (besides hidden folders and the
/// folder of generated code).
const SKIPPED_DIRS: &[&str] = &["node_modules", "target", "out", "dist", "build"];
const MAX_DEPTH: usize = 12;

/// The manifest of a modeling project (`oxychloride.json`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectManifest {
    /// The project's name (default: the name of its folder).
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Where new documents of each kind go.
    #[serde(default)]
    pub folders: ProjectFolders,
    /// The project's package registry: URIs (nsURIs, placeholders, or prefixes ending in `/`)
    /// and the files (or folders) they stand for, relative to the project folder.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub registry: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codegen: Option<CodegenSettings>,
    /// The target platform of the project's Eclipse projects (see docs/PROJECTS.md, "Eclipse
    /// projects"): bundles (jars, folders), folders of bundles (an installation's `plugins`, a
    /// bundle pool), installations, or folders with bundles further down; relative to the
    /// project, or absolute (`~/` is the home folder).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target_platform: Vec<String>,
}

/// The folders of a project, relative to it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProjectFolders {
    /// Metamodels: OCLinEcore documents and `.ecore` files.
    pub metamodels: String,
    /// Complete OCL documents.
    pub constraints: String,
    /// Models: HUTN documents and XMI files.
    pub models: String,
    /// Henshin and QVT transformations.
    pub transformations: String,
    /// Generated code (not part of the project's documents).
    pub generated: String,
}

impl Default for ProjectFolders {
    fn default() -> Self {
        ProjectFolders {
            metamodels: "metamodels".into(),
            constraints: "constraints".into(),
            models: "models".into(),
            transformations: "transformations".into(),
            generated: "src-gen".into(),
        }
    }
}

/// How the project's code is generated.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodegenSettings {
    /// The target language (`python`, `rust`).
    pub target: String,
    /// The folder the code is written into (default: the `generated` folder).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_folder: Option<String>,
}

/// A modeling project: its folder and its manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    /// The folder containing the manifest.
    pub root: PathBuf,
    pub manifest: ProjectManifest,
}

/// The documents of a project opened in a workspace (`Workspace::open_project`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProjectLoad {
    /// The names of the documents opened, metamodels first.
    pub documents: Vec<String>,
    /// Problems of the manifest and its registry (located on the manifest).
    pub problems: Vec<Diagnostic>,
}

impl Project {
    /// Reads the project whose manifest is `path`, or is in the folder `path`.
    pub fn load(path: impl AsRef<Path>) -> Result<Project, Error> {
        let path = path.as_ref();
        let file = if path.is_dir() { path.join(MANIFEST) } else { path.to_path_buf() };
        let text = std::fs::read_to_string(&file)
            .map_err(|e| Error::NotFound(format!("cannot read {}: {e}", file.display())))?;
        let root = file.parent().map(Path::to_path_buf).unwrap_or_default();
        let root = if root.as_os_str().is_empty() { PathBuf::from(".") } else { root };
        Project::parse(&text, root).map_err(|e| Error::Invalid(format!("{}: {e}", file.display())))
    }

    /// A project from the text of its manifest; `root` is the folder it is in.
    pub fn parse(text: &str, root: impl Into<PathBuf>) -> Result<Project, String> {
        let root = root.into();
        let mut manifest: ProjectManifest = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if manifest.name.trim().is_empty() {
            let folder = std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone());
            manifest.name = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        }
        Ok(Project { root, manifest })
    }

    /// The manifest of the project `start` (a file or folder) is in: the nearest
    /// `oxychloride.json` in it or above it.
    pub fn find(start: impl AsRef<Path>) -> Option<PathBuf> {
        let start = std::fs::canonicalize(start.as_ref()).ok()?;
        start.ancestors().map(|dir| dir.join(MANIFEST)).find(|m| m.is_file())
    }

    /// The path of the manifest.
    pub fn manifest_path(&self) -> PathBuf {
        self.root.join(MANIFEST)
    }

    /// A path relative to the project (with `/`s) as a path.
    pub fn resolve(&self, relative: &str) -> PathBuf {
        let path = Path::new(relative);
        if path.is_absolute() || self.root == Path::new(".") { path.to_path_buf() } else { self.root.join(path) }
    }

    /// The project's package registry, with locations resolved against the project folder.
    pub fn registry(&self) -> Vec<RegistryEntry> {
        self.manifest
            .registry
            .iter()
            .filter(|(uri, _)| !uri.trim().is_empty())
            .map(|(uri, location)| {
                let path = self.resolve(location);
                let mut location = path.to_string_lossy().into_owned();
                // A prefix's folder keeps its trailing separator.
                if uri.ends_with('/') && !location.ends_with(['/', '\\']) {
                    location.push(std::path::MAIN_SEPARATOR);
                }
                RegistryEntry { uri: uri.clone(), location }
            })
            .collect()
    }

    /// The project's Eclipse projects (those below its folder) and their target platform.
    pub fn platform(&self) -> PlatformSettings {
        let root = std::path::absolute(&self.root).unwrap_or_else(|_| self.root.clone());
        PlatformSettings {
            workspace: vec![root.to_string_lossy().into_owned()],
            target: self
                .manifest
                .target_platform
                .iter()
                .filter(|t| !t.trim().is_empty())
                .map(|t| {
                    let t = t.trim();
                    if t.starts_with("~/") || Path::new(t).is_absolute() {
                        t.to_string()
                    } else {
                        root.join(t).to_string_lossy().into_owned()
                    }
                })
                .collect(),
        }
    }

    /// The folder the generated code goes into.
    pub fn code_folder(&self) -> PathBuf {
        let codegen = self.manifest.codegen.as_ref().and_then(|c| c.output_folder.as_deref());
        self.resolve(codegen.unwrap_or(&self.manifest.folders.generated))
    }

    /// The project's documents (files of the languages documents have), sorted by path.
    pub fn documents(&self) -> Vec<PathBuf> {
        self.documents_with(&[])
    }

    /// The project's documents, with the models of further file extensions (those bundles
    /// register, see [`Workspace::model_extensions`]).
    pub fn documents_with(&self, model_extensions: &[String]) -> Vec<PathBuf> {
        let skipped = [self.resolve(&self.manifest.folders.generated), self.code_folder()];
        let mut out = Vec::new();
        collect_documents(&self.root, 0, &skipped, model_extensions, &mut out);
        // Paths below the current directory as `metamodels/a.oclinecore`, not `./metamodels/…`.
        let mut out: Vec<PathBuf> =
            out.into_iter().map(|p| p.strip_prefix(".").map(Path::to_path_buf).unwrap_or(p)).collect();
        out.sort();
        out
    }
}

/// The language of a document file, by its extension (`None` for other files).
pub fn document_language(path: &Path) -> Option<Language> {
    document_language_with(path, &[])
}

/// The language of a document file, by its extension, where `model_extensions` are those of
/// XMI models too (`None` for other files).
pub fn document_language_with(path: &Path, model_extensions: &[String]) -> Option<Language> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if model_extensions.iter().any(|e| e.eq_ignore_ascii_case(&ext)) {
        return Some(Language::Xmi);
    }
    match ext.as_str() {
        "ocl" => Some(Language::Ocl),
        "oclinecore" => Some(Language::OclInEcore),
        "hutn" => Some(Language::Hutn),
        "xmi" => Some(Language::Xmi),
        "ecore" => Some(Language::Ecore),
        "henshin_text" => Some(Language::Henshin),
        "henshin" => Some(Language::HenshinModel),
        "qvto" => Some(Language::Qvto),
        "qvtr" => Some(Language::Qvtr),
        _ => None,
    }
}

fn collect_documents(dir: &Path, depth: usize, skipped: &[PathBuf], extensions: &[String], out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            let nested = path.join(MANIFEST).is_file();
            if depth < MAX_DEPTH
                && !nested
                && !name.starts_with('.')
                && !SKIPPED_DIRS.contains(&name.as_str())
                && !skipped.iter().any(|s| same_path(s, &path))
            {
                collect_documents(&path, depth + 1, skipped, extensions, out);
            }
        } else if let Some(language) = document_language_with(&path, extensions) {
            // An `.ecore` file written from the OCLinEcore document next to it is that document.
            if language == Language::Ecore && path.with_extension("oclinecore").is_file() {
                continue;
            }
            out.push(path);
        }
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    a == b || matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(a), Ok(b)) if a == b)
}

/// The problems of a project's Eclipse projects and target platform: target platform locations
/// that do not exist, packages whose `.ecore` files are missing, required bundles found nowhere.
fn platform_problems(platform: &PlatformInfo, problem: &dyn Fn(String) -> Diagnostic) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for t in &platform.target {
        if let Some(p) = &t.problem {
            out.push(problem(format!("targetPlatform: {p}")));
        }
    }
    for b in &platform.projects {
        for p in &b.packages {
            if let Some(message) = p.message.as_ref().filter(|_| p.status != RegistryStatus::Loaded) {
                out.push(problem(format!("{}: package '{}': {message}", b.name, p.ns_uri)));
            }
        }
        for message in &b.problems {
            out.push(problem(format!("{}: {message}", b.name)));
        }
    }
    if !platform.missing.is_empty() {
        let names: Vec<String> =
            platform.missing.iter().map(|m| format!("{} (required by {})", m.name, m.required_by)).collect();
        out.push(Diagnostic {
            severity: Severity::Warning,
            ..problem(format!(
                "{} required bundle{} {} neither a project nor in the target platform: {}",
                names.len(),
                if names.len() == 1 { "" } else { "s" },
                if names.len() == 1 { "is" } else { "are" },
                names.join(", ")
            ))
        });
    }
    out
}

/// The order documents are opened in: metamodels first (the others use them), models last.
fn opening_order(language: Language) -> u8 {
    match language {
        Language::Ecore | Language::OclInEcore => 0,
        Language::Ocl => 1,
        Language::Henshin | Language::HenshinModel | Language::Qvto | Language::Qvtr => 2,
        Language::Hutn | Language::Xmi => 3,
    }
}

impl Workspace {
    /// Opens a project: its package registry replaces the workspace's, and all its documents
    /// are opened (models included), so that `diagnostics` reports the problems of the whole
    /// project and the constraints its models violate. Documents that cannot be read are
    /// problems; the workspace's other documents stay open.
    pub fn open_project(&mut self, project: &Project) -> ProjectLoad {
        let manifest = project.manifest_path().to_string_lossy().into_owned();
        let problem = |message: String| Diagnostic {
            severity: Severity::Error,
            message,
            source: manifest.clone(),
            origin: Origin::File { path: manifest.clone() },
            range: Range::default(),
        };
        let mut load = ProjectLoad::default();
        let platform = self.set_platform(project.platform());
        load.problems.extend(platform_problems(&platform, &problem));
        for info in self.set_registry(project.registry()) {
            if let Some(message) = info.message {
                load.problems.push(problem(format!("registry: '{}': {message}", info.uri)));
            }
            load.problems.extend(info.warnings);
        }
        let extensions = self.model_extensions();
        let mut documents: Vec<(PathBuf, Language)> = project
            .documents_with(&extensions)
            .into_iter()
            .filter_map(|p| Some((p.clone(), document_language_with(&p, &extensions)?)))
            .collect();
        documents.sort_by_key(|(_, language)| opening_order(*language));
        for (path, language) in documents {
            let name = path.to_string_lossy().into_owned();
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    self.open_document_as(&name, &text, language);
                    load.documents.push(name);
                }
                Err(e) => load.problems.push(problem(format!("cannot read {name}: {e}"))),
            }
        }
        // Imports between documents resolve once all are there.
        self.reload();
        load
    }

    /// A modeling project of the Eclipse projects in a folder (see [`ImportEclipseRequest`]):
    /// its manifest (an existing one, with the target platform replaced), and the Eclipse
    /// projects found with the target platform — their packages loaded, their requirements
    /// looked up. Nothing is written, and the workspace is not used.
    pub fn import_eclipse_projects(request: &ImportEclipseRequest) -> Result<ImportedProject, Error> {
        let folder = Path::new(&request.folder);
        if !folder.is_dir() {
            return Err(Error::NotFound(format!("there is no folder '{}'", folder.display())));
        }
        let mut manifest = match Project::load(folder) {
            Ok(existing) => existing.manifest,
            Err(_) => ProjectManifest::default(),
        };
        manifest.target_platform = request.target_platform.clone();
        let mut project = Project { root: folder.to_path_buf(), manifest };
        let mut ws = Workspace::new();
        let platform = ws.set_platform(project.platform());
        if platform.projects.is_empty() {
            return Err(Error::Invalid(format!(
                "there are no Eclipse projects in '{}' (folders with a .project or a META-INF/MANIFEST.MF)",
                folder.display()
            )));
        }
        let manifest = &mut project.manifest;
        if let Some(name) = request.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
            manifest.name = name.to_string();
        } else if manifest.name.trim().is_empty() {
            manifest.name = match platform.projects.as_slice() {
                [only] => only.project.clone().unwrap_or_else(|| only.name.clone()),
                _ => {
                    let folder = std::fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf());
                    folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
                }
            };
        }
        let mut text = serde_json::to_string_pretty(&project.manifest).expect("serializable");
        text.push('\n');
        Ok(ImportedProject { manifest: ProjectFile { path: MANIFEST.to_string(), text }, platform })
    }

    /// A new document about loaded metamodels (see [`NewDocumentRequest`]).
    pub fn new_document(&self, request: &NewDocumentRequest) -> Result<String, Error> {
        let loaded = self.packages();
        let package = |qualified: &str| {
            package_info(&loaded, qualified)
                .ok_or_else(|| Error::NotFound(format!("no package '{qualified}' is loaded")))
        };
        let packages: Vec<(&str, &PackageInfo)> =
            request.packages.iter().map(|q| Ok((q.as_str(), package(q)?))).collect::<Result<_, Error>>()?;
        let name = request.name.trim();
        match request.language {
            Language::OclInEcore => {
                let package = identifier(name);
                let ns_uri = request.ns_uri.clone().unwrap_or_else(|| default_ns_uri(&package));
                check_uri(&ns_uri)?;
                Ok(templates::new_metamodel(&package, &ns_uri))
            }
            // The same metamodel, as EMF writes it.
            Language::Ecore => {
                let package = identifier(name);
                let ns_uri = request.ns_uri.clone().unwrap_or_else(|| default_ns_uri(&package));
                check_uri(&ns_uri)?;
                let mut ws = Workspace::new();
                let document = format!("{package}.oclinecore");
                ws.open_document_as(&document, &templates::new_metamodel(&package, &ns_uri), Language::OclInEcore);
                Ok(ws.export_document(&document, None)?.text)
            }
            Language::Ocl => match packages.first() {
                Some(_) => Ok(templates::new_constraints(&packages)),
                None => Err(Error::Invalid("name the packages the constraints are about".into())),
            },
            // A HUTN document has a block per root package.
            Language::Hutn => match packages.first() {
                Some((qualified, _)) => {
                    Ok(templates::new_model(package(qualified.split("::").next().unwrap_or(qualified))?))
                }
                None => Err(Error::Invalid("name the package of the model".into())),
            },
            Language::Qvto | Language::Qvtr => {
                let (Some((_, source)), Some((_, target))) = (packages.first(), packages.last()) else {
                    return Err(Error::Invalid("name the packages the transformation transforms".into()));
                };
                let name = sanitize(if name.is_empty() { "transformation" } else { name }, false);
                let roots = (root_class(source), root_class(target));
                Ok(if request.language == Language::Qvto {
                    templates::new_qvto(&name, source, target, roots)
                } else {
                    templates::new_qvtr(&name, source, target, roots)
                })
            }
            Language::Henshin | Language::HenshinModel => {
                Err(Error::Invalid("new Henshin transformations are written by editTransformation (setImports)".into()))
            }
            Language::Xmi => Err(Error::Invalid("new XMI models are written by their clients".into())),
        }
    }

    /// A new modeling project (see [`NewProjectRequest`]): its manifest and documents, checked
    /// to be free of errors. Nothing is written, and the workspace is not used.
    pub fn new_project(request: &NewProjectRequest) -> Result<NewProject, Error> {
        let name = request.name.trim();
        if name.is_empty() {
            return Err(Error::Invalid("the project needs a name".into()));
        }
        let package = identifier(name);
        let ns_uri =
            request.ns_uri.clone().filter(|u| !u.trim().is_empty()).unwrap_or_else(|| default_ns_uri(&package));
        check_uri(&ns_uri)?;
        if let Some(target) = &request.codegen
            && oxy_codegen::target(target).is_none()
        {
            return Err(Error::Invalid(format!("unknown code generation target '{target}'")));
        }
        let include = request.include.clone().unwrap_or_else(|| {
            vec![
                Language::Ocl,
                Language::Hutn,
                Language::Xmi,
                Language::Henshin,
                Language::HenshinModel,
                Language::Qvto,
                Language::Qvtr,
            ]
        });
        let has = |l: Language| include.contains(&l);
        let folders = ProjectFolders::default();
        let at = |folder: &str, file: String| format!("{folder}/{file}");

        // The documents are written, opened and converted in a workspace of their own, at
        // paths below a root that does not exist (references between them are relative).
        let root = PathBuf::from(if cfg!(windows) { r"C:\oxychloride-project" } else { "/oxychloride-project" });
        let absolute = |relative: &str| root.join(relative).to_string_lossy().into_owned();
        let mut ws = Workspace::new();
        let mut documents: Vec<ProjectFile> = Vec::new();
        let mut registry = BTreeMap::new();
        // The first error of the documents (a name that is a keyword of one of the languages,
        // say), else `e`: without the root.
        let failure = |ws: &Workspace, e: Option<Error>| -> Error {
            let root = root.to_string_lossy();
            let relative = |s: &str| s.replace(&format!("{root}{}", std::path::MAIN_SEPARATOR), "");
            let diagnostics = ws.diagnostics(None);
            match (diagnostics.iter().find(|d| d.severity == Severity::Error), e) {
                (Some(d), _) => Error::Invalid(format!(
                    "cannot create a project named '{name}': {}: {}",
                    relative(&d.source),
                    d.message
                )),
                (None, Some(e)) => Error::Invalid(relative(&e.to_string())),
                (None, None) => Error::Invalid(format!("cannot create a project named '{name}'")),
            }
        };
        let export = |ws: &Workspace, from: &str, to: &str| {
            ws.export_document(&absolute(from), Some(&absolute(to))).map(|r| r.text).map_err(|e| failure(ws, Some(e)))
        };

        let metamodel_text = templates::project_metamodel(&package, &ns_uri);
        let oclinecore = at(&folders.metamodels, format!("{package}.oclinecore"));
        ws.open_document_as(&absolute(&oclinecore), &metamodel_text, Language::OclInEcore);
        match request.metamodel {
            MetamodelFormat::OclInEcore => documents.push(ProjectFile { path: oclinecore, text: metamodel_text }),
            MetamodelFormat::Ecore => {
                let ecore = at(&folders.metamodels, format!("{package}.ecore"));
                let text = export(&ws, &oclinecore, &ecore)?;
                ws.close_document(&absolute(&oclinecore));
                ws.load_ecore_str(&text, &absolute(&ecore)).map_err(|e| failure(&ws, Some(e)))?;
                registry.insert(ns_uri.clone(), ecore.clone());
                documents.push(ProjectFile { path: ecore, text });
            }
        }
        let add = |ws: &mut Workspace, documents: &mut Vec<ProjectFile>, path: String, text: String, language| {
            ws.open_document_as(&absolute(&path), &text, language);
            documents.push(ProjectFile { path, text });
        };
        if has(Language::Ocl) {
            let path = at(&folders.constraints, format!("{package}.ocl"));
            add(&mut ws, &mut documents, path, templates::project_constraints(&package, &ns_uri), Language::Ocl);
        }
        let hutn = at(&folders.models, "example.hutn".into());
        if has(Language::Hutn) || has(Language::Xmi) {
            ws.open_document_as(&absolute(&hutn), &templates::project_model(&package, &ns_uri), Language::Hutn);
        }
        if has(Language::Xmi) {
            let xmi = at(&folders.models, "example.xmi".into());
            let text = export(&ws, &hutn, &xmi)?;
            if !has(Language::Hutn) {
                ws.close_document(&absolute(&hutn));
            }
            add(&mut ws, &mut documents, xmi, text, Language::Xmi);
        }
        if has(Language::Hutn) {
            documents.push(ProjectFile { path: hutn, text: templates::project_model(&package, &ns_uri) });
        }
        let henshin_text = at(&folders.transformations, format!("{package}.henshin_text"));
        if has(Language::Henshin) || has(Language::HenshinModel) {
            ws.open_document_as(&absolute(&henshin_text), &templates::project_henshin(&package), Language::Henshin);
        }
        if has(Language::HenshinModel) {
            let model = at(&folders.transformations, format!("{package}.henshin"));
            let text = export(&ws, &henshin_text, &model)?;
            if !has(Language::Henshin) {
                ws.close_document(&absolute(&henshin_text));
            }
            add(&mut ws, &mut documents, model, text, Language::HenshinModel);
        }
        if has(Language::Henshin) {
            documents.push(ProjectFile { path: henshin_text, text: templates::project_henshin(&package) });
        }
        if has(Language::Qvto) {
            let path = at(&folders.transformations, "copy.qvto".into());
            add(&mut ws, &mut documents, path, templates::project_qvto(&package, &ns_uri), Language::Qvto);
        }
        if has(Language::Qvtr) {
            let path = at(&folders.transformations, "copy.qvtr".into());
            add(&mut ws, &mut documents, path, templates::project_qvtr(&package, &ns_uri), Language::Qvtr);
        }

        ws.reload();
        if ws.diagnostics(None).iter().any(|d| d.severity == Severity::Error) {
            return Err(failure(&ws, None));
        }

        let manifest = ProjectManifest {
            name: name.to_string(),
            description: request.description.clone().unwrap_or_default(),
            folders: folders.clone(),
            registry,
            codegen: request.codegen.clone().map(|target| CodegenSettings { target, output_folder: None }),
            target_platform: Vec::new(),
        };
        let mut manifest_text = serde_json::to_string_pretty(&manifest).expect("serializable");
        manifest_text.push('\n');
        // Documents in a stable order: by folder, as listed in the manifest.
        documents.sort_by_key(|d| {
            let rank = [&folders.metamodels, &folders.constraints, &folders.models, &folders.transformations]
                .iter()
                .position(|f| d.path.starts_with(&format!("{f}/")))
                .unwrap_or(usize::MAX);
            (rank, d.path.clone())
        });
        let mut files = vec![ProjectFile { path: MANIFEST.into(), text: manifest_text }];
        let readme = templates::readme(&manifest, &documents);
        files.extend(documents);
        files.push(ProjectFile { path: "README.md".into(), text: readme });
        files.push(ProjectFile { path: ".vscode/extensions.json".into(), text: templates::EXTENSIONS.into() });
        Ok(NewProject { package, ns_uri, files })
    }
}

/// The package with a qualified name (`a::b`) among `packages` and their subpackages.
fn package_info<'a>(packages: &'a [PackageInfo], qualified: &str) -> Option<&'a PackageInfo> {
    let (first, rest) = qualified.split_once("::").map_or((qualified, None), |(f, r)| (f, Some(r)));
    let p = packages.iter().find(|p| p.name == first)?;
    match rest {
        Some(rest) => package_info(&p.subpackages, rest),
        None => Some(p),
    }
}

/// The class of a package a model's root object is best made of: a concrete class no
/// containment of the package holds (else the first concrete class).
fn root_class(package: &PackageInfo) -> Option<String> {
    let concrete: Vec<&ClassifierInfo> = package
        .classifiers
        .iter()
        .filter(|c| c.kind == ClassifierKind::Class && !c.is_abstract && !c.is_interface)
        .collect();
    let contained: Vec<&str> = package
        .classifiers
        .iter()
        .flat_map(|c| &c.features)
        .filter(|f| f.containment)
        .map(|f| simple_type(&f.ty))
        .collect();
    let by_name = |name: &str| package.classifiers.iter().find(|c| c.name == name);
    let held = |c: &ClassifierInfo| {
        let mut pending = vec![c];
        let mut seen = Vec::new();
        while let Some(c) = pending.pop() {
            if seen.contains(&c.name) {
                continue;
            }
            seen.push(c.name.clone());
            if contained.contains(&c.name.as_str()) {
                return true;
            }
            pending.extend(c.super_types.iter().filter_map(|s| by_name(s.rsplit("::").next().unwrap_or(s))));
        }
        false
    };
    concrete.iter().find(|c| !held(c)).or(concrete.first()).map(|c| c.name.clone())
}

/// The class a feature's type names: `Element` of `OrderedSet(shop::Element)`.
fn simple_type(ty: &str) -> &str {
    let inner = ty.rsplit_once('(').map_or(ty, |(_, t)| t.trim_end_matches(')'));
    inner.rsplit("::").next().unwrap_or(inner)
}

/// An identifier made of a name: `My Shop` → `my_shop`.
pub fn identifier(name: &str) -> String {
    sanitize(name, true)
}

/// An identifier made of a name, in lower case or keeping the case.
fn sanitize(name: &str, lower: bool) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(if lower { c.to_ascii_lowercase() } else { c });
        } else if !out.is_empty() && !out.ends_with('_') {
            out.push('_');
        }
    }
    let out = out.trim_end_matches('_').to_string();
    match out.chars().next() {
        None => "model".into(),
        Some(c) if c.is_ascii_digit() => format!("m{out}"),
        Some(_) => out,
    }
}

fn default_ns_uri(package: &str) -> String {
    format!("http://example.org/{package}")
}

/// URIs are written into documents between quotes of either kind.
fn check_uri(uri: &str) -> Result<(), Error> {
    if uri.trim().is_empty() || uri.contains(['\'', '"', '\\']) || uri.chars().any(char::is_whitespace) {
        return Err(Error::Invalid(format!("'{uri}' cannot be an nsURI")));
    }
    Ok(())
}

mod templates {
    //! The texts of new projects and documents.

    use super::*;

    pub(super) const EXTENSIONS: &str = r#"{
  "recommendations": ["palladio.oxychloride", "redhat.vscode-xml"]
}
"#;

    pub(super) fn project_metamodel(package: &str, ns_uri: &str) -> String {
        format!(
            r#"-- The metamodel of {package} in OCLinEcore: classes with their features, operations and
-- constraints. A starting point: rename, add and remove elements as you need, as text or in
-- the diagram ("Open Metamodel Diagram").

package {package} : {package} = '{ns_uri}' {{
  -- The root of a model: it contains all elements.
  class Model {{
    attribute name : String[1];
    property elements#model : Element[*] {{ ordered, composes }};

    invariant UniqueNames('Two elements are named alike'): elements->isUnique(name);
  }}

  class Element {{
    attribute name : String[1];
    property model#elements : Model[?];
    -- Other elements this one depends on.
    property dependencies : Element[*];

    -- The elements this one depends on, directly or not.
    operation allDependencies() : Element[*] {{
      body: dependencies->closure(dependencies)->asSet();
    }}

    invariant HasName: name.size() > 0;
  }}
}}
"#
        )
    }

    pub(super) fn project_constraints(package: &str, ns_uri: &str) -> String {
        format!(
            r#"-- Further constraints on {package} models in Complete OCL: they are checked like the
-- invariants of the metamodel, live in the model editors and by "Validate Models".

import '{ns_uri}'

package {package}

context Element
-- An element must not depend on itself, not even indirectly.
inv Acyclic('Element ' + name + ' depends on itself'): not allDependencies()->includes(self)

endpackage
"#
        )
    }

    pub(super) fn project_model(package: &str, ns_uri: &str) -> String {
        format!(
            r#"// A {package} model in HUTN, checked against the metamodel and the constraints as you type.
// Add objects with "Create Instance" in the Metamodels view, or "Add Child…" in the Models view.

@Spec {{
  metamodel "{package}" {{
    nsUri: "{ns_uri}"
  }}
}}

{package} {{
  Model "example" {{
    name: "example"
    elements:
      Element "first" {{ name: "first" }},
      Element "second" {{ name: "second" dependencies: Element "first" }}
  }}
}}
"#
        )
    }

    pub(super) fn project_henshin(package: &str) -> String {
        format!(
            r#"// In-place transformations of {package} models in Henshin's textual syntax. Open a model,
// then "Open Transformation" on this file to apply the rules step by step, or open the rule
// diagram.

ePackageImport {package}

// Adds an element to a model, unless the model has an element of that name.
rule addElement(IN name:EString) {{
	graph {{
		node model:Model
		create node element:Element {{
			name=name
		}}
		edges [(create model->element:elements)]
		forbid node other:Element {{
			name=name
		}}
		edges [(forbid model->other:elements)]
	}}
}}

// Makes an element depend on another.
rule addDependency(IN from:EString, IN to:EString) {{
	graph {{
		node source:Element {{
			name=from
		}}
		node target:Element {{
			name=to
		}}
		edges [(create source->target:dependencies)]
	}}
}}
"#
        )
    }

    pub(super) fn project_qvto(package: &str, ns_uri: &str) -> String {
        let alias = package.to_ascii_uppercase();
        format!(
            r#"-- A transformation of {package} models in QVT Operational Mappings: it copies a model,
-- element by element. Run it on a model with "Run QVT Transformation…".

modeltype {alias} uses '{ns_uri}';

transformation copy(in source : {alias}, out target : {alias});

main() {{
	source.rootObjects()[{alias}::Model]->map toModel();
}}

mapping {alias}::Model::toModel() : {alias}::Model {{
	name := self.name;
	elements := self.elements->map toElement();
}}

-- The copies of the dependencies may not exist yet: `late` resolves them at the end.
mapping {alias}::Element::toElement() : {alias}::Element {{
	name := self.name;
	dependencies := self.dependencies.late resolveone({alias}::Element);
}}
"#
        )
    }

    pub(super) fn project_qvtr(package: &str, ns_uri: &str) -> String {
        format!(
            r#"-- A transformation of {package} models in QVT Relations: run in the direction of `target`,
-- it creates a model with the elements of the source model; in check mode, it reports the
-- elements the target lacks. Run it on a model with "Run QVT Transformation…".

import {package} : '{ns_uri}';

transformation copy(source : {package}, target : {package}) {{
	-- Objects with the same values of these properties are the same object: enforcing a
	-- relation finds them instead of creating others.
	key {package}::Model {{ name }};
	key {package}::Element {{ model, name }};

	top relation ModelToModel {{
		n : String;
		checkonly domain source s : {package}::Model {{ name = n }};
		enforce domain target t : {package}::Model {{ name = n }};
		where {{ ElementToElement(s, t); }}
	}}

	relation ElementToElement {{
		n : String;
		checkonly domain source s : {package}::Model {{ elements = e : {package}::Element {{ name = n }} }};
		enforce domain target t : {package}::Model {{ elements = f : {package}::Element {{ name = n }} }};
	}}
}}
"#
        )
    }

    pub(super) fn readme(manifest: &ProjectManifest, documents: &[ProjectFile]) -> String {
        let f = &manifest.folders;
        let in_folder = |folder: &str| -> Vec<&str> {
            documents.iter().filter_map(|d| d.path.strip_prefix(&format!("{folder}/"))).collect()
        };
        let list = |files: Vec<&str>| files.iter().map(|f| format!("`{f}`")).collect::<Vec<_>>().join(", ");
        let mut rows = vec![
            (f.metamodels.as_str(), "The metamodel (Ecore, with its OCL)", list(in_folder(&f.metamodels))),
            (f.constraints.as_str(), "Further constraints in Complete OCL", list(in_folder(&f.constraints))),
            (f.models.as_str(), "Models in HUTN and XMI, checked against all constraints", list(in_folder(&f.models))),
            (
                f.transformations.as_str(),
                "Henshin graph transformations and QVT model transformations",
                list(in_folder(&f.transformations)),
            ),
        ];
        rows.retain(|(_, _, files)| !files.is_empty());
        let mut text = format!("# {}\n\n", manifest.name);
        if !manifest.description.is_empty() {
            text.push_str(&format!("{}\n\n", manifest.description));
        }
        text.push_str(
            "A modeling project of [Oxychloride](https://github.com/palladio/oxychloride): \
             `oxychloride.json` describes it (its folders, its package registry, how code is generated).\n\n",
        );
        text.push_str("| Folder | Contents | Files |\n|---|---|---|\n");
        for (folder, what, files) in &rows {
            text.push_str(&format!("| `{folder}/` | {what} | {files} |\n"));
        }
        text.push_str(&format!("| `{}/` | Generated code | |\n", f.generated));
        let metamodel = documents.first().map(|d| d.path.as_str()).unwrap_or_default();
        let model = documents.iter().map(|d| d.path.as_str()).find(|p| p.ends_with(".hutn") || p.ends_with(".xmi"));
        let target = manifest.codegen.as_ref().map(|c| c.target.as_str()).unwrap_or("python");
        text.push_str(&format!(
            r#"
## In VS Code

With the Oxychloride extension, the **Modeling Projects** view (in the Oxychloride bar) shows
the project's documents by kind, and creates new ones in the right folder. Documents are checked
as you type, models against every constraint of the metamodel and the Complete OCL documents;
**Validate Project** checks all models of the project at once. The metamodel and Henshin's rules
can be edited as diagrams too.

## On the command line

```sh
oxy check                                  # every document of the project, models included
oxy generate {metamodel} -t {target} -o {generated}
"#,
            generated = f.generated
        ));
        let qvt = documents.iter().find(|d| d.path.ends_with(".qvto") || d.path.ends_with(".qvtr"));
        if let (Some(model), Some(qvt)) = (model, qvt) {
            text.push_str(&format!("oxy qvt {} -e {metamodel} --in source={model} --hutn\n", qvt.path));
        }
        text.push_str("```\n");
        text
    }

    pub(super) fn new_metamodel(package: &str, ns_uri: &str) -> String {
        format!(
            r#"-- The metamodel {package} in OCLinEcore: classes with their features, operations and
-- constraints (invariants, operation bodies, pre- and postconditions, derivations).

package {package} : {package} = '{ns_uri}' {{
  class {class} {{
    attribute name : String;
  }}
}}
"#,
            class = pascal_case(package)
        )
    }

    /// Complete OCL about packages (by qualified name), with a block for the first.
    pub(super) fn new_constraints(packages: &[(&str, &PackageInfo)]) -> String {
        let names: Vec<&str> = packages.iter().map(|(_, p)| p.name.as_str()).collect();
        let mut text = format!(
            "-- Constraints on {} models in Complete OCL: they are checked like the invariants of the\n-- metamodel.\n\n",
            names.join(" and ")
        );
        for (_, p) in packages {
            text.push_str(&format!("import '{}'\n", p.ns_uri));
        }
        let (qualified, p) = packages[0];
        let class =
            p.classifiers.iter().find(|c| c.kind == ClassifierKind::Class).map_or("SomeClass", |c| c.name.as_str());
        text.push_str(&format!(
            r#"
package {qualified}

-- context {class}
-- inv Name('a message'): a Boolean expression about self
-- def: name(parameters) : Type = an expression

endpackage
"#,
        ));
        text
    }

    pub(super) fn new_model(package: &PackageInfo) -> String {
        format!(
            r#"// A model of {name} in HUTN: objects are `Class "identifier" {{ feature: value ... }}`.

@Spec {{
  metamodel "{name}" {{
    nsUri: "{ns_uri}"
  }}
}}

{name} {{
}}
"#,
            name = package.name,
            ns_uri = package.ns_uri
        )
    }

    pub(super) fn new_qvto(
        name: &str,
        source: &PackageInfo,
        target: &PackageInfo,
        (from, to): (Option<String>, Option<String>),
    ) -> String {
        let source_alias = source.name.to_ascii_uppercase();
        let target_alias = target.name.to_ascii_uppercase();
        let mut text = format!(
            "-- {name}: a transformation of {} models into {} models in QVT Operational Mappings.\n\n",
            source.name, target.name
        );
        text.push_str(&format!("modeltype {source_alias} uses '{}';\n", source.ns_uri));
        if target.ns_uri != source.ns_uri {
            text.push_str(&format!("modeltype {target_alias} uses '{}';\n", target.ns_uri));
        }
        text.push_str(&format!(
            "\ntransformation {name}(in source : {source_alias}, out target : {target_alias});\n\n"
        ));
        match (from, to) {
            (Some(from), Some(to)) => text.push_str(&format!(
                r#"main() {{
	source.rootObjects()[{source_alias}::{from}]->map to{to}();
}}

mapping {source_alias}::{from}::to{to}() : {target_alias}::{to} {{
}}
"#
            )),
            _ => text.push_str("main() {\n}\n"),
        }
        text
    }

    pub(super) fn new_qvtr(
        name: &str,
        source: &PackageInfo,
        target: &PackageInfo,
        (from, to): (Option<String>, Option<String>),
    ) -> String {
        let (s, t) = (&source.name, &target.name);
        let mut text = format!("-- {name}: a transformation between {s} models and {t} models in QVT Relations.\n\n");
        text.push_str(&format!("import {s} : '{}';\n", source.ns_uri));
        if target.ns_uri != source.ns_uri {
            text.push_str(&format!("import {t} : '{}';\n", target.ns_uri));
        }
        text.push_str(&format!("\ntransformation {name}(source : {s}, target : {t}) {{\n"));
        if let (Some(from), Some(to)) = (from, to) {
            text.push_str(&format!(
                r#"	top relation {from}To{to} {{
		checkonly domain source a : {s}::{from} {{}};
		enforce domain target b : {t}::{to} {{}};
	}}
"#
            ));
        }
        text.push_str("}\n");
        text
    }

    fn pascal_case(name: &str) -> String {
        name.split('_')
            .filter(|s| !s.is_empty())
            .map(|s| {
                let mut chars = s.chars();
                chars.next().map(|c| c.to_ascii_uppercase().to_string() + chars.as_str()).unwrap_or_default()
            })
            .collect()
    }
}
