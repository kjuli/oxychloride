//! The package registry: URIs standing for metamodel files, like EMF's package registry and
//! URI map (see [`RegistryEntry`]).
//!
//! Registered files are loaded right away, before any other `.ecore` file, so their packages
//! are known everywhere — to documents and models naming them by nsURI, to imports, and to
//! other `.ecore` files. A registered URI need not be reachable: references to it (`href`s of
//! `.ecore` files, `import`s, `xsi:schemaLocation`s) are read from the registered file instead,
//! which is then known by that URI too. Prefix entries map whole folders and are used on
//! demand, like [`Workspace::map_uri`].

use std::path::Path;

use oxy_ecore::ecore_loader::{self, canonical_uri};
use oxy_ecore::{EcoreLoad, LoadError, Metamodel};

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

/// Where URIs are read from: the registry's entries and the URI mappings.
#[derive(Debug, Clone, Default)]
pub(crate) struct Locator {
    /// Registered URIs and their files.
    exact: Vec<(String, String)>,
    /// URI prefixes and their folders.
    prefixes: Vec<(String, String)>,
}

impl Locator {
    /// The file or path a URI is read from, if it is registered or under a mapped prefix.
    pub(crate) fn locate(&self, uri: &str) -> Option<String> {
        if let Some((_, location)) = self.exact.iter().find(|(u, _)| u == uri) {
            return Some(location.clone());
        }
        self.locate_prefixed(uri)
    }

    /// The path of a URI under a mapped prefix (the longest one).
    pub(crate) fn locate_prefixed(&self, uri: &str) -> Option<String> {
        let (from, to) =
            self.prefixes.iter().filter(|(from, _)| uri.starts_with(from.as_str())).max_by_key(|(f, _)| f.len())?;
        Some(canonical_uri(&Path::new(to).join(&uri[from.len()..])))
    }

    /// Reads a referenced resource (see [`ecore_loader::Fetch`]): from its registered location, else from disk.
    pub(crate) fn fetch(&self, uri: &str) -> Option<(String, String)> {
        match self.locate(uri) {
            Some(path) => std::fs::read_to_string(&path).ok().map(|text| (canonical_uri(Path::new(&path)), text)),
            None => ecore_loader::read_from_disk(uri),
        }
    }

    /// Loads an `.ecore` file into `mm`, reading the files it refers to through the registry.
    pub(crate) fn load_file(&self, mm: &mut Metamodel, path: &Path) -> Result<EcoreLoad, LoadError> {
        ecore_loader::load_file_with(mm, path, &mut |uri| self.fetch(uri))
    }

    /// Loads `.ecore` content into `mm`, reading the files it refers to through the registry.
    pub(crate) fn load_str(&self, mm: &mut Metamodel, text: &str, uri: &str) -> Result<EcoreLoad, LoadError> {
        ecore_loader::load_with(mm, text, uri, &mut |u| self.fetch(u))
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
    }
}
