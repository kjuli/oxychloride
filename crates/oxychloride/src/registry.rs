//! The package registry: URIs standing for metamodel files, like EMF's package registry and
//! URI map (see [`RegistryEntry`]).
//!
//! Registered files are loaded right away, before any other `.ecore` file, so their packages
//! are known everywhere — to documents and models naming them by nsURI, to imports, and to
//! other `.ecore` files. A registered URI need not be reachable: references to it (`href`s of
//! `.ecore` files, `import`s, `xsi:schemaLocation`s) are read from the registered file instead,
//! which is then known by that URI too. Prefix entries map whole folders and are used on
//! demand, like [`Workspace::map_uri`].

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use oxy_ecore::ecore_loader::{self, canonical_uri};
use oxy_ecore::{EcoreLoad, LoadError, Metamodel};

use crate::archive;
use crate::platform::Platform;
use crate::protocol::*;
use crate::workspace::Workspace;

/// The state of a registry entry after loading.
#[derive(Debug, Clone)]
pub(crate) struct EntryState {
    status: RegistryStatus,
    message: Option<String>,
    /// The loaded file, in `base_mm`.
    resource: Option<usize>,
    warnings: Vec<Diagnostic>,
}

/// Where URIs are read from: the registry's entries, the URI mappings, and the Eclipse
/// projects and target platform.
#[derive(Debug, Clone, Default)]
pub(crate) struct Locator {
    /// Registered URIs and their files.
    exact: Vec<(String, String)>,
    /// URI prefixes and their folders.
    prefixes: Vec<(String, String)>,
    platform: Arc<Platform>,
    /// The texts of open `.ecore` documents, by path: they stand for the files.
    overlays: Arc<HashMap<String, String>>,
}

impl Locator {
    /// The file or path a URI is read from, if it is registered, under a mapped prefix, or
    /// known to the platform (`platform:/…` URIs, packages of bundles, their URI mappings).
    pub(crate) fn locate(&self, uri: &str) -> Option<String> {
        if let Some((_, location)) = self.exact.iter().find(|(u, _)| u == uri) {
            return Some(location.clone());
        }
        self.locate_prefixed(uri).or_else(|| self.platform.locate(uri))
    }

    /// The path of a URI under a mapped prefix (the longest one).
    pub(crate) fn locate_prefixed(&self, uri: &str) -> Option<String> {
        let (from, to) =
            self.prefixes.iter().filter(|(from, _)| uri.starts_with(from.as_str())).max_by_key(|(f, _)| f.len())?;
        let rest = &uri[from.len()..];
        if archive::split_uri(to).is_some() || to.ends_with("!/") {
            return Some(format!("{to}{rest}"));
        }
        Some(canonical_uri(&Path::new(to).join(rest)))
    }

    /// The text at a location (a path or an archive URI); an open `.ecore` document's text
    /// for its file.
    pub(crate) fn read(&self, location: &str) -> Option<String> {
        if let Some(text) = self.overlays.get(location) {
            return Some(text.clone());
        }
        if archive::split_uri(location).is_some() {
            return archive::read_uri(location);
        }
        std::fs::read_to_string(location).ok()
    }

    /// Whether there is a file at a location (a path or an archive URI).
    pub(crate) fn exists(&self, location: &str) -> bool {
        match archive::split_uri(location) {
            Some((jar, entry)) => archive::open_cached(&jar).is_ok_and(|a| a.contains(entry)),
            None => self.overlays.contains_key(location) || Path::new(location).is_file(),
        }
    }

    /// Reads a referenced resource (see [`ecore_loader::Fetch`]): from its registered location, else from disk.
    pub(crate) fn fetch(&self, uri: &str) -> Option<(String, String)> {
        match self.locate(uri) {
            Some(location) => {
                let text = self.read(&location)?;
                let location = if archive::split_uri(&location).is_some() {
                    location
                } else {
                    canonical_uri(Path::new(&location))
                };
                Some((location, text))
            }
            None if archive::split_uri(uri).is_some() => archive::read_uri(uri).map(|t| (uri.to_string(), t)),
            None => match self.overlays.get(uri) {
                Some(text) => Some((uri.to_string(), text.clone())),
                None => ecore_loader::read_from_disk(uri),
            },
        }
    }

    /// The URIs a file is known by in Eclipse (`platform:/resource/…`, `platform:/plugin/…`).
    pub(crate) fn logical_uris(&self, location: &str) -> Vec<String> {
        self.platform.logical_uris(location)
    }

    /// Loads an `.ecore` file (a path, or an archive URI) into `mm`, reading the files it
    /// refers to through the registry; relative references resolve against the URIs the file
    /// is known by in Eclipse first, as in EMF.
    pub(crate) fn load_file(&self, mm: &mut Metamodel, path: &Path) -> Result<EcoreLoad, LoadError> {
        let location = path.to_string_lossy();
        let location =
            if archive::split_uri(&location).is_some() { location.into_owned() } else { canonical_uri(path) };
        let logical = self.logical_uris(&location);
        let mut fetch = |uri: &str| self.fetch(uri);
        if mm.resource_of_uri(&location).is_none()
            && (archive::split_uri(&location).is_some() || self.overlays.contains_key(&location))
        {
            let text = self.read(&location).ok_or_else(|| LoadError::new(&location, "cannot read file".to_string()))?;
            return ecore_loader::load_as(mm, &text, &location, &logical, &mut fetch);
        }
        ecore_loader::load_file_as(mm, &location, &logical, &mut fetch)
    }

    /// Loads `.ecore` content into `mm`, reading the files it refers to through the registry.
    pub(crate) fn load_str(&self, mm: &mut Metamodel, text: &str, uri: &str) -> Result<EcoreLoad, LoadError> {
        let logical = self.logical_uris(uri);
        ecore_loader::load_as(mm, text, uri, &logical, &mut |u| self.fetch(u))
    }
}

impl Workspace {
    /// Replaces the package registry (see [`RegistryEntry`]) and reloads the metamodels, models
    /// and document imports accordingly. Relative locations are resolved against the working
    /// directory. Returns the state of the entries.
    pub fn set_registry(&mut self, entries: Vec<RegistryEntry>) -> Vec<RegistryInfo> {
        let entries: Vec<RegistryEntry> = entries
            .into_iter()
            .filter(|e| !e.uri.trim().is_empty())
            .map(|e| RegistryEntry {
                uri: e.uri.trim().to_string(),
                location: canonical_uri(&std::path::absolute(&e.location).unwrap_or_else(|_| e.location.into())),
            })
            .collect();
        if entries != self.registry {
            self.registry = entries;
            self.reload();
        }
        self.registry()
    }

    /// Registers one URI (replacing an entry for the same URI), see [`Workspace::set_registry`].
    pub fn register(&mut self, uri: &str, location: &str) -> RegistryInfo {
        let mut entries: Vec<RegistryEntry> = self.registry.iter().filter(|e| e.uri != uri).cloned().collect();
        entries.push(RegistryEntry { uri: uri.to_string(), location: location.to_string() });
        let infos = self.set_registry(entries);
        infos.into_iter().find(|i| i.uri == uri.trim()).expect("the entry was just registered")
    }

    /// Removes the entry for a URI; returns whether there was one.
    pub fn unregister(&mut self, uri: &str) -> bool {
        let entries: Vec<RegistryEntry> = self.registry.iter().filter(|e| e.uri != uri).cloned().collect();
        let found = entries.len() != self.registry.len();
        self.set_registry(entries);
        found
    }

    /// The entries of the package registry, with absolute locations.
    pub fn registry_entries(&self) -> &[RegistryEntry] {
        &self.registry
    }

    /// The entries of the package registry and their state.
    pub fn registry(&self) -> Vec<RegistryInfo> {
        let mm = &self.base_mm;
        let ecore = mm.package(mm.ecore_package()).resource;
        let resource_info = |r: usize| {
            let res = &mm.resources()[r];
            RegisteredResource {
                uri: res.uri.clone(),
                aliases: res.aliases.clone(),
                packages: res
                    .roots
                    .iter()
                    .map(|p| RegisteredPackage {
                        name: mm.package(*p).name.clone(),
                        ns_uri: mm.package(*p).ns_uri.clone(),
                    })
                    .collect(),
            }
        };
        self.registry
            .iter()
            .zip(&self.registry_state)
            .map(|(entry, state)| {
                let mut imports: Vec<usize> = Vec::new();
                let mut pending: Vec<usize> = state.resource.into_iter().collect();
                while let Some(r) = pending.pop() {
                    for i in mm.referenced_resources(r) {
                        if i != ecore && Some(i) != state.resource && !imports.contains(&i) {
                            imports.push(i);
                            pending.push(i);
                        }
                    }
                }
                RegistryInfo {
                    uri: entry.uri.clone(),
                    location: entry.location.clone(),
                    status: state.status,
                    message: state.message.clone(),
                    resource: state.resource.map(resource_info),
                    imports: imports.into_iter().map(resource_info).collect(),
                    warnings: state.warnings.clone(),
                }
            })
            .collect()
    }

    /// Where URIs are read from (registry entries, then URI mappings).
    pub(crate) fn locator(&self) -> Locator {
        let (prefixes, exact): (Vec<&RegistryEntry>, Vec<&RegistryEntry>) =
            self.registry.iter().partition(|e| e.is_prefix());
        Locator {
            exact: exact.into_iter().map(|e| (e.uri.clone(), e.location.clone())).collect(),
            prefixes: prefixes
                .into_iter()
                .map(|e| (e.uri.clone(), e.location.clone()))
                .chain(self.uri_mappings.iter().cloned())
                .collect(),
            platform: self.platform.clone(),
            overlays: self.overlays.clone(),
        }
    }

    /// Loads the registered files (into `base_mm` and `mm`), recording the entries' state.
    pub(crate) fn load_registry(&mut self) {
        let mut states = Vec::new();
        for entry in self.registry.clone() {
            let path = Path::new(&entry.location);
            let state =
                |status, message: Option<String>| EntryState { status, message, resource: None, warnings: Vec::new() };
            let state = if entry.is_prefix() {
                if path.is_dir() {
                    state(RegistryStatus::Folder, None)
                } else {
                    state(RegistryStatus::Missing, Some(format!("there is no folder '{}'", entry.location)))
                }
            } else if !path.is_file() {
                state(RegistryStatus::Missing, Some(format!("there is no file '{}'", entry.location)))
            } else {
                match self.load_metamodel_file(path) {
                    Ok(load) => {
                        self.base_mm.add_resource_alias(load.resource, &entry.uri);
                        let uri = self.base_mm.resources()[load.resource].uri.clone();
                        if let Some(r) = self.mm.resource_of_uri(&uri) {
                            self.mm.add_resource_alias(r, &entry.uri);
                        }
                        EntryState {
                            status: RegistryStatus::Loaded,
                            message: None,
                            resource: Some(load.resource),
                            warnings: load.warnings.iter().map(crate::workspace::load_diagnostic).collect(),
                        }
                    }
                    Err(e) => state(RegistryStatus::Error, Some(e.to_string())),
                }
            };
            states.push(state);
        }
        self.registry_state = states;
        self.load_platform_packages();
    }

    /// Loads the packages the Eclipse projects register (their `plugin.xml`), after the
    /// registry's files (which take precedence), recording their state.
    fn load_platform_packages(&mut self) {
        let mut states = Vec::new();
        let locator = self.locator();
        for (ns_uri, file) in self.platform.project_packages() {
            let state =
                |status, message: Option<String>| EntryState { status, message, resource: None, warnings: Vec::new() };
            let state = if let Some(p) = self.base_mm.package_by_ns_uri(&ns_uri) {
                let resource = self.base_mm.package(p).resource;
                EntryState { resource: Some(resource), ..state(RegistryStatus::Loaded, None) }
            } else if !locator.exists(&file) {
                state(RegistryStatus::Missing, Some(format!("there is no file '{file}'")))
            } else {
                match self.load_metamodel_file(Path::new(&file)) {
                    Ok(load) => EntryState {
                        status: RegistryStatus::Loaded,
                        message: None,
                        resource: Some(load.resource),
                        warnings: load.warnings.iter().map(crate::workspace::load_diagnostic).collect(),
                    },
                    Err(e) => state(RegistryStatus::Error, Some(e.to_string())),
                }
            };
            states.push((ns_uri, state));
        }
        self.platform_state = states;
    }

    /// Sets the Eclipse projects and the target platform (see [`PlatformSettings`]): finds the
    /// projects below the workspace folders again, reads their manifests and `plugin.xml`s and
    /// those of the bundles they require, and reloads. Returns their state.
    pub fn set_platform(&mut self, settings: PlatformSettings) -> PlatformInfo {
        let platform = Platform::new(settings);
        let changed = !(platform.is_empty() && self.platform.is_empty());
        self.platform = Arc::new(platform);
        if changed {
            self.reload();
        }
        self.platform()
    }

    /// The Eclipse projects, the bundles of the target platform they require, and the state of
    /// the packages they register.
    pub fn platform(&self) -> PlatformInfo {
        let packages = |ns_uri: &str| match self.platform_state.iter().find(|(u, _)| u == ns_uri) {
            Some((_, state)) => (state.status, state.message.clone()),
            None => (RegistryStatus::Loaded, Some("registered by another project".to_string())),
        };
        self.platform.info(&packages)
    }

    /// The file extensions of models that bundles register (`.repository`, …): such files are
    /// XMI models.
    pub fn model_extensions(&self) -> Vec<String> {
        self.platform.model_extensions()
    }
}
