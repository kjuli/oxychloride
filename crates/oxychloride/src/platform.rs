//! Eclipse projects and the target platform (see docs/PROJECTS.md, "Eclipse projects"): what an
//! Eclipse workspace knows about its projects and the bundles of its target platform, so that
//! EMF's URIs mean what they mean in Eclipse — without Eclipse:
//!
//! * `platform:/resource/<project>/<path>` is a file of a workspace project (named by its
//!   `.project`);
//! * `platform:/plugin/<bundle>/<path>` is a file of a bundle (by its symbolic name): of the
//!   workspace project that is the bundle, else of the target platform's bundle — a folder or a
//!   jar (`archive:file:…!/<path>`), its highest version;
//! * bundles register packages by nsURI (`org.eclipse.emf.ecore.generated_package`, whose
//!   `.ecore` is found through its genmodel, and `dynamic_package`), map URIs
//!   (`org.eclipse.emf.ecore.uri_mapping`, e.g. `pathmap://PCM_MODELS/`) and name the file
//!   extensions of their models (`extension_parser`).
//!
//! The packages of workspace projects are loaded with the workspace (they are its metamodels);
//! those of the target platform when something refers to them. Of the target platform, only
//! the bundles that the workspace projects require (`Require-Bundle`, transitively) are read,
//! as in Eclipse, where a project sees its dependencies; all bundles are reachable by
//! `platform:/plugin` URIs.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use oxy_ecore::ecore_loader::{canonical_uri, resolve_uri};

use crate::archive;
use crate::protocol::*;

/// Folders not searched for Eclipse projects and bundles (besides hidden ones).
const SKIPPED_DIRS: &[&str] = &["node_modules", "target", "bin", "out", "dist", "build"];
const MAX_DEPTH: usize = 8;
/// How deep target platform locations that are not bundle folders are searched.
const TARGET_DEPTH: usize = 4;

/// Extensions `extension_parser`s register that are not models of a bundle's own (or are
/// documents of other languages).
const NOT_MODEL_EXTENSIONS: &[&str] = &[
    "*",
    "xml",
    "xmi",
    "ecore",
    "emof",
    "genmodel",
    "henshin",
    "henshin_text",
    "ocl",
    "oclinecore",
    "hutn",
    "qvto",
    "qvtr",
];

/// Where a bundle is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BundleLocation {
    Folder(PathBuf),
    Jar(PathBuf),
}

impl BundleLocation {
    /// Where a file of the bundle is: a path, or an archive URI.
    pub fn file(&self, relative: &str) -> String {
        let relative = relative.trim_start_matches('/');
        match self {
            BundleLocation::Folder(dir) => canonical_uri(&dir.join(relative)),
            BundleLocation::Jar(jar) => archive::uri(jar, relative),
        }
    }

    /// The text of a file of the bundle.
    pub fn read(&self, relative: &str) -> Option<String> {
        let relative = relative.trim_start_matches('/');
        match self {
            BundleLocation::Folder(dir) => std::fs::read_to_string(dir.join(relative)).ok(),
            BundleLocation::Jar(jar) => archive::open_cached(jar).ok()?.read_to_string(relative).ok(),
        }
    }

    /// The location as shown: a folder's or jar's path.
    pub fn display(&self) -> String {
        match self {
            BundleLocation::Folder(p) | BundleLocation::Jar(p) => p.to_string_lossy().into_owned(),
        }
    }
}

/// A package a bundle registers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundlePackage {
    pub ns_uri: String,
    /// The `.ecore` file, relative to the bundle (`None`: the genmodel names none).
    pub path: Option<String>,
    /// The genmodel of a generated package, relative to the bundle.
    pub genmodel: Option<String>,
}

/// An Eclipse project of the workspace, or a bundle of the target platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bundle {
    /// The project's name (`.project`), for workspace projects.
    pub project: Option<String>,
    /// The bundle's symbolic name (`Bundle-SymbolicName`), if it is one.
    pub symbolic_name: Option<String>,
    pub version: String,
    pub location: BundleLocation,
    /// `Require-Bundle`: names, and whether optional.
    pub requires: Vec<(String, bool)>,
    pub packages: Vec<BundlePackage>,
    /// `org.eclipse.emf.ecore.uri_mapping`: source → target (as written).
    pub uri_mappings: Vec<(String, String)>,
    /// File extensions of models (`extension_parser` types).
    pub extensions: Vec<String>,
    /// Problems reading the bundle's metadata.
    pub problems: Vec<String>,
}

impl Bundle {
    /// The URI the bundle's files have (`platform:/plugin/<name>/`, or for a project without a
    /// manifest `platform:/resource/<project>/`).
    pub fn base_uri(&self) -> String {
        match (&self.symbolic_name, &self.project) {
            (Some(name), _) => format!("platform:/plugin/{name}/"),
            (None, Some(project)) => format!("platform:/resource/{project}/"),
            (None, None) => String::new(),
        }
    }

    /// The bundle's name: its symbolic name, else its project's name.
    pub fn name(&self) -> &str {
        self.symbolic_name.as_deref().or(self.project.as_deref()).unwrap_or_default()
    }

    /// Reads a bundle's metadata: `META-INF/MANIFEST.MF`, `plugin.xml` and the genmodels its
    /// generated packages name.
    fn read(location: BundleLocation, project: Option<String>) -> Bundle {
        let mut bundle = Bundle { project, ..Bundle::read_nothing(location) };
        if let Some(manifest) = bundle.location.read("META-INF/MANIFEST.MF") {
            let headers = manifest_headers(&manifest);
            let header =
                |name: &str| headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str());
            bundle.symbolic_name =
                header("Bundle-SymbolicName").and_then(|v| clauses(v).into_iter().next()).map(|c| c.name);
            bundle.version = header("Bundle-Version").unwrap_or_default().trim().to_string();
            bundle.requires = header("Require-Bundle")
                .map(clauses)
                .unwrap_or_default()
                .into_iter()
                .map(|c| (c.name, c.optional))
                .collect();
        }
        if let Some(text) = bundle.location.read("plugin.xml") {
            bundle.read_plugin_xml(&text);
        }
        bundle
    }

    /// A bundle at `location` whose metadata is not read.
    fn read_nothing(location: BundleLocation) -> Bundle {
        Bundle {
            project: None,
            symbolic_name: None,
            version: String::new(),
            location,
            requires: Vec::new(),
            packages: Vec::new(),
            uri_mappings: Vec::new(),
            extensions: Vec::new(),
            problems: Vec::new(),
        }
    }

    /// The nsURIs a `plugin.xml` registers packages for (their files not looked up).
    fn read_plugin_xml_packages(&mut self, text: &str) {
        let Ok(root) = oxy_ecore::xml::parse(text, "plugin.xml") else { return };
        for extension in root.children_named("extension") {
            let point = extension.attr("point").unwrap_or_default();
            for el in &extension.children {
                let is_package = matches!(
                    (point, el.local_name()),
                    ("org.eclipse.emf.ecore.generated_package", "package")
                        | ("org.eclipse.emf.ecore.dynamic_package", "resource")
                );
                if let Some(ns_uri) = el.attr("uri").filter(|_| is_package) {
                    self.packages.push(BundlePackage { ns_uri: ns_uri.to_string(), path: None, genmodel: None });
                }
            }
        }
    }

    fn read_plugin_xml(&mut self, text: &str) {
        let root = match oxy_ecore::xml::parse(text, "plugin.xml") {
            Ok(root) => root,
            Err(e) => {
                self.problems.push(format!("plugin.xml: {}", e.message));
                return;
            }
        };
        let mut genmodels: HashMap<String, Vec<(String, Vec<String>)>> = HashMap::new();
        for extension in root.children_named("extension") {
            let point = extension.attr("point").unwrap_or_default();
            for el in &extension.children {
                let attr = |name: &str| el.attr(name).map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
                match (point, el.local_name()) {
                    ("org.eclipse.emf.ecore.generated_package", "package") => {
                        let Some(ns_uri) = attr("uri") else { continue };
                        let genmodel = attr("genModel");
                        let path = genmodel.as_ref().and_then(|g| self.ecore_of_genmodel(g, &ns_uri, &mut genmodels));
                        self.packages.push(BundlePackage { ns_uri, path, genmodel });
                    }
                    ("org.eclipse.emf.ecore.dynamic_package", "resource") => {
                        if let (Some(ns_uri), Some(location)) = (attr("uri"), attr("location")) {
                            self.packages.push(BundlePackage { ns_uri, path: Some(location), genmodel: None });
                        }
                    }
                    ("org.eclipse.emf.ecore.uri_mapping", "mapping") => {
                        if let (Some(source), Some(target)) = (attr("source"), attr("target")) {
                            self.uri_mappings.push((source, target));
                        }
                    }
                    ("org.eclipse.emf.ecore.extension_parser", "parser") => {
                        if let Some(ty) = attr("type")
                            && !NOT_MODEL_EXTENSIONS.contains(&ty.to_ascii_lowercase().as_str())
                            && !self.extensions.contains(&ty)
                        {
                            self.extensions.push(ty);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// The `.ecore` file (relative to the bundle) that defines `ns_uri`, of those a genmodel
    /// generates code for (`foreignModel`, the `ecorePackage`s of its `genPackages`). The
    /// genmodel's files and their nsURIs are read once per genmodel.
    fn ecore_of_genmodel(
        &mut self,
        genmodel: &str,
        ns_uri: &str,
        cache: &mut HashMap<String, Vec<(String, Vec<String>)>>,
    ) -> Option<String> {
        if !cache.contains_key(genmodel) {
            let files = self.genmodel_files(genmodel);
            cache.insert(genmodel.to_string(), files);
        }
        let files = &cache[genmodel];
        files
            .iter()
            .find(|(_, uris)| uris.iter().any(|u| u == ns_uri))
            .or_else(|| files.first())
            .map(|(path, _)| path.clone())
    }

    /// The `.ecore` files a genmodel names (relative to the bundle) and their packages' nsURIs.
    fn genmodel_files(&mut self, genmodel: &str) -> Vec<(String, Vec<String>)> {
        let Some(text) = self.location.read(genmodel) else {
            self.problems.push(format!("cannot read the genmodel '{genmodel}'"));
            return Vec::new();
        };
        let Ok(root) = oxy_ecore::xml::parse(&text, genmodel) else {
            self.problems.push(format!("cannot parse the genmodel '{genmodel}'"));
            return Vec::new();
        };
        let mut relative: Vec<String> = Vec::new();
        let foreign = root.attr("foreignModel").map(|v| v.split_whitespace().map(str::to_string).collect());
        let foreign: Vec<String> =
            foreign.unwrap_or_else(|| root.children_named("foreignModel").map(|f| f.text.trim().to_string()).collect());
        relative.extend(foreign.into_iter().filter(|f| f.ends_with(".ecore")));
        for p in root.children_named("genPackages") {
            if let Some((file, _)) = p.attr("ecorePackage").and_then(|h| h.split_once('#')) {
                relative.push(file.to_string());
            }
        }
        // Relative to the genmodel, as paths of the bundle.
        let base = format!("bundle:/{genmodel}");
        let mut files: Vec<(String, Vec<String>)> = Vec::new();
        for file in relative {
            if oxy_ecore::ecore_loader::has_scheme(&file) {
                continue;
            }
            let path = resolve_uri(&base, &file).trim_start_matches("bundle:/").to_string();
            if files.iter().any(|(p, _)| *p == path) {
                continue;
            }
            let uris = self.location.read(&path).map(|t| ns_uris(&t)).unwrap_or_default();
            files.push((path, uris));
        }
        files
    }
}

/// The nsURIs of the packages of an `.ecore` text.
fn ns_uris(text: &str) -> Vec<String> {
    fn collect(el: &oxy_ecore::xml::Element, out: &mut Vec<String>) {
        if matches!(el.local_name(), "EPackage" | "eSubpackages")
            && let Some(u) = el.attr("nsURI")
        {
            out.push(u.to_string());
        }
        for c in &el.children {
            if matches!(c.local_name(), "EPackage" | "eSubpackages") {
                collect(c, out);
            }
        }
    }
    let mut out = Vec::new();
    if let Ok(root) = oxy_ecore::xml::parse(text, "") {
        collect(&root, &mut out);
    }
    out
}

/// The headers of a `MANIFEST.MF` (continuation lines joined).
fn manifest_headers(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix(' ') {
            if let Some((_, v)) = out.last_mut() {
                v.push_str(rest);
            }
        } else if let Some((k, v)) = line.split_once(':') {
            out.push((k.trim().to_string(), v.trim_start().to_string()));
        }
    }
    out
}

struct Clause {
    name: String,
    optional: bool,
}

/// The clauses of a manifest header (`a;bundle-version="[1.0,2.0)",b;resolution:=optional`).
fn clauses(value: &str) -> Vec<Clause> {
    let mut parts = Vec::new();
    let (mut current, mut quoted) = (String::new(), false);
    for c in value.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                current.push(c);
            }
            ',' if !quoted => parts.push(std::mem::take(&mut current)),
            c => current.push(c),
        }
    }
    parts.push(current);
    parts
        .into_iter()
        .filter_map(|p| {
            let mut segments = p.split(';').map(str::trim);
            let name = segments.next().filter(|n| !n.is_empty())?.to_string();
            let optional = segments.any(|s| s.replace(' ', "") == "resolution:=optional");
            Some(Clause { name, optional })
        })
        .collect()
}

/// An OSGi version (`major.minor.micro.qualifier`), for picking a bundle's highest version.
fn version_key(version: &str) -> (u64, u64, u64, String) {
    let mut parts = version.splitn(4, '.');
    let mut number = || parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let (a, b, c) = (number(), number(), number());
    (a, b, c, parts.next().unwrap_or_default().to_string())
}

/// The symbolic name and version of a bundle file or folder named `<name>_<version>`.
fn split_bundle_name(file: &str) -> Option<(&str, &str)> {
    let stem = file.strip_suffix(".jar").unwrap_or(file);
    let i = stem.rfind('_')?;
    let (name, version) = (&stem[..i], &stem[i + 1..]);
    (!name.is_empty() && version.starts_with(|c: char| c.is_ascii_digit())).then_some((name, version))
}

/// A target platform's bundle: where its highest version is.
#[derive(Debug, Clone)]
struct TargetBundle {
    version: String,
    location: BundleLocation,
}

/// The Eclipse projects of a workspace and its target platform.
#[derive(Debug, Clone, Default)]
pub struct Platform {
    pub settings: PlatformSettings,
    /// The Eclipse projects of the workspace folders.
    pub projects: Vec<Bundle>,
    /// The target platform's bundles by symbolic name.
    target: HashMap<String, TargetBundle>,
    /// The bundles of the target platform the projects require (transitively), read.
    pub required: Vec<Bundle>,
    /// Required bundles found nowhere: (requiring bundle, required bundle).
    pub missing: Vec<(String, String)>,
    /// The target platform's locations: how many bundles each has, or why it has none.
    pub locations: Vec<(String, Result<usize, String>)>,
    /// The nsURIs all bundles of the target platform register, and the bundles: like Eclipse's
    /// global package registry. Read when an nsURI is not registered by the projects or the
    /// bundles they require.
    registered: OnceLock<HashMap<String, String>>,
}

impl Platform {
    /// Finds the Eclipse projects below the workspace folders and indexes the target platform.
    pub fn new(settings: PlatformSettings) -> Platform {
        let mut platform = Platform { settings: settings.clone(), ..Platform::default() };
        let mut seen = HashSet::new();
        for folder in &settings.workspace {
            let mut found = Vec::new();
            find_projects(Path::new(folder), 0, &mut found);
            for dir in found {
                let canonical = std::fs::canonicalize(&dir).unwrap_or(dir);
                if seen.insert(canonical.clone()) {
                    let project = project_name(&canonical);
                    platform.projects.push(Bundle::read(BundleLocation::Folder(canonical), project));
                }
            }
        }
        platform.projects.sort_by(|a, b| a.name().cmp(b.name()));
        for location in &settings.target {
            let path = expand_home(location);
            let mut bundles = Vec::new();
            let result = if path.exists() {
                index_target(&path, 0, &mut bundles);
                Ok(bundles.len())
            } else {
                Err(format!("there is no file or folder '{}'", path.display()))
            };
            for (name, version, location) in bundles {
                let newer = platform.target.get(&name).is_none_or(|t| version_key(&version) > version_key(&t.version));
                if newer {
                    platform.target.insert(name, TargetBundle { version, location });
                }
            }
            platform.locations.push((location.clone(), result));
        }
        platform.resolve_requirements();
        platform
    }

    /// Reads the target bundles the projects require, transitively.
    fn resolve_requirements(&mut self) {
        let mut pending: Vec<(String, String, bool)> = self
            .projects
            .iter()
            .flat_map(|p| p.requires.iter().map(|(r, optional)| (p.name().to_string(), r.clone(), *optional)))
            .collect();
        let mut done: HashSet<String> = self.projects.iter().filter_map(|p| p.symbolic_name.clone()).collect();
        while let Some((from, name, optional)) = pending.pop() {
            if done.contains(&name) {
                continue;
            }
            match self.target.get(&name) {
                Some(t) => {
                    done.insert(name.clone());
                    let mut bundle = Bundle::read(t.location.clone(), None);
                    if bundle.symbolic_name.is_none() {
                        bundle.symbolic_name = Some(name.clone());
                    }
                    bundle.version = t.version.clone();
                    pending.extend(bundle.requires.iter().map(|(r, o)| (name.clone(), r.clone(), *o)));
                    self.required.push(bundle);
                }
                None if !optional && !is_system_bundle(&name) && !self.missing.iter().any(|(_, m)| *m == name) => {
                    self.missing.push((from, name));
                }
                None => {}
            }
        }
        self.required.sort_by(|a, b| a.name().cmp(b.name()));
        self.missing.sort_by(|a, b| a.1.cmp(&b.1));
    }

    /// Whether there is anything: projects or a target platform.
    pub fn is_empty(&self) -> bool {
        self.projects.is_empty() && self.target.is_empty()
    }

    /// The workspace project named `name` (by its project name).
    fn project(&self, name: &str) -> Option<&Bundle> {
        self.projects.iter().find(|p| p.project.as_deref() == Some(name))
    }

    /// The bundle with the symbolic name `name`: a workspace project, else of the target platform.
    fn bundle_location(&self, name: &str) -> Option<BundleLocation> {
        self.projects
            .iter()
            .find(|p| p.symbolic_name.as_deref() == Some(name))
            .or_else(|| self.projects.iter().find(|p| p.symbolic_name.is_none() && p.project.as_deref() == Some(name)))
            .map(|p| p.location.clone())
            .or_else(|| self.target.get(name).map(|t| t.location.clone()))
    }

    /// Where a URI is read from: a `platform:/resource` or `platform:/plugin` URI's file, a
    /// registered package's `.ecore` (by nsURI), or a mapped URI's target.
    pub fn locate(&self, uri: &str) -> Option<String> {
        self.locate_depth(uri, 0)
    }

    fn locate_depth(&self, uri: &str, depth: usize) -> Option<String> {
        if let Some(rest) = uri.strip_prefix("platform:/resource/") {
            let (name, path) = rest.split_once('/').unwrap_or((rest, ""));
            let name = decode(name);
            return self.project(&name).map(|p| p.location.file(&decode(path)));
        }
        if let Some(rest) = uri.strip_prefix("platform:/plugin/") {
            let (name, path) = rest.split_once('/').unwrap_or((rest, ""));
            return self.bundle_location(&decode(name)).map(|l| l.file(&decode(path)));
        }
        if let Some((bundle, package)) = self.package(uri) {
            return package.path.as_ref().map(|p| bundle.location.file(p));
        }
        if let Some(name) = self.registered().get(uri) {
            let bundle = Bundle::read(self.target[name].location.clone(), None);
            let package = bundle.packages.iter().find(|p| p.ns_uri == uri)?;
            return package.path.as_ref().map(|p| bundle.location.file(p));
        }
        if depth > 4 {
            return None;
        }
        // URI mappings, the longest source first.
        let mapped = self
            .bundles()
            .flat_map(|b| b.uri_mappings.iter().map(move |(s, t)| (b, s, t)))
            .filter(
                |(_, source, _)| if source.ends_with('/') { uri.starts_with(source.as_str()) } else { uri == *source },
            )
            .max_by_key(|(_, source, _)| source.len());
        let (bundle, source, target) = mapped?;
        let target = resolve_uri(&bundle.base_uri(), target);
        self.locate_depth(&format!("{target}{}", &uri[source.len()..]), depth + 1)
    }

    /// The nsURIs the target platform's bundles register (see [`Platform::registered`]).
    fn registered(&self) -> &HashMap<String, String> {
        self.registered.get_or_init(|| {
            let mut out = HashMap::new();
            for (name, t) in &self.target {
                let Some(text) = t.location.read("plugin.xml") else { continue };
                if !text.contains("org.eclipse.emf.ecore.generated_package")
                    && !text.contains("org.eclipse.emf.ecore.dynamic_package")
                {
                    continue;
                }
                let mut bundle = Bundle { packages: Vec::new(), ..Bundle::read_nothing(t.location.clone()) };
                bundle.read_plugin_xml_packages(&text);
                for ns_uri in bundle.packages.into_iter().map(|p| p.ns_uri) {
                    out.entry(ns_uri).or_insert_with(|| name.clone());
                }
            }
            out
        })
    }

    /// The workspace projects, then the required bundles of the target platform.
    pub fn bundles(&self) -> impl Iterator<Item = &Bundle> {
        self.projects.iter().chain(&self.required)
    }

    /// The package registered under `ns_uri`, and its bundle (workspace projects first).
    pub fn package(&self, ns_uri: &str) -> Option<(&Bundle, &BundlePackage)> {
        self.bundles().find_map(|b| b.packages.iter().find(|p| p.ns_uri == ns_uri).map(|p| (b, p)))
    }

    /// The packages of the workspace projects (loaded with the workspace): nsURI and file.
    pub fn project_packages(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        for project in &self.projects {
            for p in &project.packages {
                if let Some(path) = &p.path {
                    let file = project.location.file(path);
                    if !out.iter().any(|(u, _)| *u == p.ns_uri) {
                        out.push((p.ns_uri.clone(), file));
                    }
                }
            }
        }
        out
    }

    /// The URIs a file is known by in Eclipse (`platform:/resource/<project>/…` and
    /// `platform:/plugin/<bundle>/…`), given its path or archive URI.
    pub fn logical_uris(&self, location: &str) -> Vec<String> {
        let mut out = Vec::new();
        if let Some((jar, entry)) = archive::split_uri(location) {
            for b in self.bundles() {
                if b.location == BundleLocation::Jar(jar.clone()) {
                    out.push(format!("{}{entry}", b.base_uri()));
                }
            }
            return out;
        }
        let path = Path::new(location);
        // The innermost project containing the file.
        let project = self
            .projects
            .iter()
            .filter_map(|p| match &p.location {
                BundleLocation::Folder(dir) => path.strip_prefix(dir).ok().map(|rel| (p, dir, rel)),
                BundleLocation::Jar(_) => None,
            })
            .max_by_key(|(_, dir, _)| dir.as_os_str().len());
        if let Some((p, _, rel)) = project {
            let rel = rel.to_string_lossy().replace('\\', "/");
            if let Some(name) = &p.project {
                out.push(format!("platform:/resource/{}/{rel}", encode(name)));
            }
            if let Some(name) = &p.symbolic_name {
                out.push(format!("platform:/plugin/{name}/{rel}"));
            }
        }
        out
    }

    /// The file extensions of models that the projects and their required bundles register.
    pub fn model_extensions(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for b in self.bundles() {
            for e in &b.extensions {
                if !out.iter().any(|x| x.eq_ignore_ascii_case(e)) {
                    out.push(e.clone());
                }
            }
        }
        out
    }

    /// How many of `names` the target platform has.
    fn provides(&self, names: &[String]) -> usize {
        names.iter().filter(|n| self.target.contains_key(*n)).count()
    }
}

/// Bundles that need not be in the target platform: the OSGi framework's (which no target
/// platform lists as a file), and EMF's own, whose metamodel (`Ecore.ecore`, also as
/// `platform:/plugin/org.eclipse.emf.ecore/model/Ecore.ecore`) the engine has built in.
fn is_system_bundle(name: &str) -> bool {
    matches!(name, "system.bundle" | "org.eclipse.osgi" | "org.eclipse.emf.ecore" | "org.eclipse.emf.common")
}

fn decode(s: &str) -> String {
    match archive::file_uri_path(&format!("file:{s}")) {
        Some(p) => p.to_string_lossy().into_owned(),
        None => s.to_string(),
    }
}

fn encode(s: &str) -> String {
    s.replace('%', "%25").replace(' ', "%20").replace('#', "%23")
}

/// `~/…` as a path below the home folder.
pub fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => home().map(|h| h.join(rest)).unwrap_or_else(|| PathBuf::from(path)),
        None => PathBuf::from(path),
    }
}

/// A path below the home folder as `~/…` (as manifests write it, to work for others too).
pub fn contract_home(path: &str) -> String {
    match home().and_then(|h| Path::new(path).strip_prefix(&h).ok().map(Path::to_path_buf)) {
        Some(rest) => format!("~/{}", rest.to_string_lossy().replace('\\', "/")),
        None => path.to_string(),
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from)
}

/// The name of the Eclipse project in `dir` (its `.project`'s `<name>`).
fn project_name(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(".project")).ok()?;
    let root = oxy_ecore::xml::parse(&text, ".project").ok()?;
    let name = root.children_named("name").next()?.text.trim().to_string();
    (!name.is_empty()).then_some(name)
}

fn is_project_folder(dir: &Path) -> bool {
    dir.join(".project").is_file() || dir.join("META-INF").join("MANIFEST.MF").is_file()
}

/// Folders below `dir` (and `dir` itself) that are Eclipse projects or bundles.
fn find_projects(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if is_project_folder(dir) {
        out.push(dir.to_path_buf());
    }
    if depth >= MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            !name.starts_with('.') && !SKIPPED_DIRS.contains(&name.as_str())
        })
        .map(|e| e.path())
        .collect();
    dirs.sort();
    for d in dirs {
        find_projects(&d, depth + 1, out);
    }
}

/// The bundles of a target platform location: a bundle (jar or folder), a folder of bundles
/// (`plugins`, a bundle pool), an installation or bundle pool (with `plugins`), or a folder
/// with bundles further down (checkouts of projects).
fn index_target(path: &Path, depth: usize, out: &mut Vec<(String, String, BundleLocation)>) {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if path.is_file() {
        if name.ends_with(".jar") {
            add_target_bundle(BundleLocation::Jar(path.to_path_buf()), &name, out);
        }
        return;
    }
    if path.join("META-INF").join("MANIFEST.MF").is_file() {
        add_target_bundle(BundleLocation::Folder(path.to_path_buf()), &name, out);
        return;
    }
    // An Eclipse installation on macOS.
    let app = path.join("Contents").join("Eclipse");
    if app.join("plugins").is_dir() {
        return index_target(&app, depth, out);
    }
    let plugins = path.join("plugins");
    if plugins.is_dir() && name != "plugins" {
        index_target(&plugins, depth, out);
        return;
    }
    if depth > TARGET_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(path) else { return };
    let mut entries: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            !name.starts_with('.') && !SKIPPED_DIRS.contains(&name.as_str())
        })
        .map(|e| e.path())
        .collect();
    entries.sort();
    for e in entries {
        let is_jar = e.extension().is_some_and(|x| x == "jar");
        if is_jar || e.is_dir() {
            index_target(&e, depth + 1, out);
        }
    }
}

fn add_target_bundle(location: BundleLocation, file: &str, out: &mut Vec<(String, String, BundleLocation)>) {
    match split_bundle_name(file) {
        Some((name, version)) => out.push((name.to_string(), version.to_string(), location)),
        // Not named `<name>_<version>`: the manifest says.
        None => {
            let Some(manifest) = location.read("META-INF/MANIFEST.MF") else { return };
            let headers = manifest_headers(&manifest);
            let header = |k: &str| headers.iter().find(|(h, _)| h.eq_ignore_ascii_case(k)).map(|(_, v)| v.clone());
            if let Some(name) = header("Bundle-SymbolicName").and_then(|v| clauses(&v).into_iter().next()) {
                out.push((name.name, header("Bundle-Version").unwrap_or_default().trim().to_string(), location));
            }
        }
    }
}

impl Platform {
    /// What the engine shows of the platform.
    pub fn info(&self, packages: &dyn Fn(&str) -> (RegistryStatus, Option<String>)) -> PlatformInfo {
        let bundle_info = |b: &Bundle, workspace: bool| EclipseBundleInfo {
            name: b.name().to_string(),
            project: b.project.clone(),
            symbolic_name: b.symbolic_name.clone(),
            version: (!b.version.is_empty()).then(|| b.version.clone()),
            location: b.location.display(),
            packages: b
                .packages
                .iter()
                .map(|p| {
                    let location = p.path.as_ref().map(|path| b.location.file(path));
                    let (status, message) = match &location {
                        None => (
                            RegistryStatus::Missing,
                            Some(match &p.genmodel {
                                Some(g) => format!("the genmodel '{g}' names no .ecore file"),
                                None => "no .ecore file is named".to_string(),
                            }),
                        ),
                        Some(_) if !workspace => (RegistryStatus::Folder, None),
                        Some(_) => packages(&p.ns_uri),
                    };
                    EclipsePackageInfo { ns_uri: p.ns_uri.clone(), location, status, message }
                })
                .collect(),
            requires: b
                .requires
                .iter()
                .map(|(name, optional)| RequiredBundle {
                    name: name.clone(),
                    optional: *optional,
                    location: self.bundle_location(name).map(|l| l.display()),
                })
                .collect(),
            uri_mappings: b
                .uri_mappings
                .iter()
                .map(|(source, target)| UriMappingInfo { source: source.clone(), target: target.clone() })
                .collect(),
            model_extensions: b.extensions.clone(),
            problems: b.problems.clone(),
        };
        PlatformInfo {
            settings: self.settings.clone(),
            projects: self.projects.iter().map(|b| bundle_info(b, true)).collect(),
            required: self.required.iter().map(|b| bundle_info(b, false)).collect(),
            missing: self
                .missing
                .iter()
                .map(|(from, name)| MissingBundle { name: name.clone(), required_by: from.clone() })
                .collect(),
            target: self
                .locations
                .iter()
                .map(|(location, result)| TargetLocationInfo {
                    location: location.clone(),
                    bundles: *result.as_ref().unwrap_or(&0),
                    problem: result.as_ref().err().cloned(),
                })
                .collect(),
            model_extensions: self.model_extensions(),
        }
    }
}

/// Eclipse projects in a folder, for importing them: the projects, and the target platforms
/// found on this machine that have the bundles they require, the best first.
pub fn scan(folder: &Path) -> EclipseScan {
    let workspace =
        Platform::new(PlatformSettings { workspace: vec![folder.to_string_lossy().into_owned()], target: Vec::new() });
    let required: Vec<String> = workspace.missing.iter().map(|(_, n)| n.clone()).collect();
    let mut candidates: Vec<TargetCandidate> = detect_targets()
        .into_iter()
        .filter_map(|(location, kind)| {
            let platform = Platform::new(PlatformSettings {
                workspace: Vec::new(),
                target: vec![location.to_string_lossy().into_owned()],
            });
            let provides = platform.provides(&required);
            (!platform.target.is_empty()).then(|| TargetCandidate {
                location: location.to_string_lossy().into_owned(),
                kind,
                bundles: platform.target.len(),
                provides,
            })
        })
        .collect();
    candidates.sort_by(|a, b| b.provides.cmp(&a.provides).then(b.bundles.cmp(&a.bundles)));
    let info = workspace.info(&|_| (RegistryStatus::Folder, None));
    EclipseScan { projects: info.projects, required, targets: candidates }
}

/// Target platforms on this machine: the bundle pools and installations of Eclipse (as Oomph
/// records them), the PDE target platforms of their recent workspaces, and the usual places.
pub fn detect_targets() -> Vec<(PathBuf, String)> {
    let mut out: Vec<(PathBuf, String)> = Vec::new();
    let mut add = |path: PathBuf, kind: &str| {
        if path.is_dir() && !out.iter().any(|(p, _)| *p == path) {
            out.push((path, kind.to_string()));
        }
    };
    let Some(home) = home() else { return Vec::new() };
    let mut agents: Vec<PathBuf> = vec![home.join(".p2")];
    if let Ok(text) = std::fs::read_to_string(home.join(".eclipse/org.eclipse.oomph.p2/agents.info")) {
        agents.extend(text.lines().map(str::trim).filter(|l| !l.is_empty()).map(PathBuf::from));
    }
    let mut installations: Vec<PathBuf> = Vec::new();
    for agent in &agents {
        if let Ok(text) = std::fs::read_to_string(agent.join("pools.info")) {
            for pool in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
                add(PathBuf::from(pool).join("plugins"), "bundle pool");
            }
        }
        if let Ok(text) = std::fs::read_to_string(agent.join("profiles.info")) {
            // `<id>|<type>|<bundle pool>|<installation folder>|`
            for line in text.lines() {
                if let Some(folder) = line.split('|').nth(3).filter(|f| !f.is_empty()) {
                    installations.push(PathBuf::from(folder));
                }
            }
        }
        add(agent.join("pool").join("plugins"), "bundle pool");
    }
    for apps in [PathBuf::from("/Applications"), home.join("Applications")] {
        for entry in std::fs::read_dir(&apps).into_iter().flatten().flatten() {
            if entry.file_name().to_string_lossy().to_ascii_lowercase().starts_with("eclipse") {
                installations.push(entry.path().join("Contents").join("Eclipse"));
            }
        }
    }
    for root in [home.join("eclipse"), home.join("Eclipse")] {
        for entry in std::fs::read_dir(&root).into_iter().flatten().flatten() {
            installations.push(entry.path().join("eclipse"));
            installations.push(entry.path().join("Eclipse.app").join("Contents").join("Eclipse"));
        }
    }
    for installation in installations {
        // The PDE target platforms of the installation's recent workspaces.
        let prefs = installation.join("configuration/.settings/org.eclipse.ui.ide.prefs");
        if let Ok(text) = std::fs::read_to_string(prefs)
            && let Some(line) = text.lines().find_map(|l| l.strip_prefix("RECENT_WORKSPACES="))
        {
            for workspace in line.split("\\n").map(str::trim).filter(|w| !w.is_empty()) {
                let pool = Path::new(workspace).join(".metadata/.plugins/org.eclipse.pde.core/.bundle_pool/plugins");
                add(pool, &format!("target platform of the Eclipse workspace {workspace}"));
            }
        }
        add(installation.join("plugins"), "Eclipse installation");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_manifests() {
        let headers = manifest_headers(
            "Bundle-SymbolicName: a.b;singleton:=true\nRequire-Bundle: c;bundle-version=\"[1.0,\n 2.0)\",\n d;resolution:=optional\n",
        );
        let requires = &headers.iter().find(|(k, _)| k == "Require-Bundle").unwrap().1;
        let clauses = clauses(requires);
        let names: Vec<(&str, bool)> = clauses.iter().map(|c| (c.name.as_str(), c.optional)).collect();
        assert_eq!(names, [("c", false), ("d", true)]);
        assert_eq!(
            split_bundle_name("de.uka.ipd.sdq.stoex_6.0.0.202605041122.jar"),
            Some(("de.uka.ipd.sdq.stoex", "6.0.0.202605041122"))
        );
        assert_eq!(split_bundle_name("org.foo_bar"), None);
        assert!(version_key("6.0.0.202610010004") > version_key("6.0.0.202605041122"));
        assert!(version_key("10.0.0") > version_key("9.9.9"));
    }
}
