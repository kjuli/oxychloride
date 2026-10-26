//! QVT transformations as documents: Operational Mappings (`.qvto`) and Relations (`.qvtr`).
//!
//! A QVT document is compiled against the workspace's metamodels — its model types and
//! imports name packages by nsURI, by a URI of the package registry, or by a metamodel file
//! relative to the document (loaded when the document is opened) — and with the OCL of the
//! workspace, so its expressions can call operations with OCL bodies and read derived
//! features. It changes neither metamodels nor models: diagnostics, hovers and completion
//! come from compiling it, and `runQvt` runs it on a copy of the workspace's models and
//! returns the models it produced or changed as text, without writing files.

use std::path::Path;

use oxy_ecore::{Metamodel, Model, ObjectId, PackageId, hutn, xmi_writer};
use oxy_qvt::{Binding, Compiled, Direction, RunOptions};

use crate::protocol::*;
use crate::workspace::{Compiled as OclCompiled, Error, Workspace};

/// The QVT language of a document language.
fn qvt_language(language: Language) -> Option<oxy_qvt::Language> {
    match language {
        Language::Qvto => Some(oxy_qvt::Language::Operational),
        Language::Qvtr => Some(oxy_qvt::Language::Relational),
        _ => None,
    }
}

/// The URIs a QVT document's model types (QVTo) or imports (QVTr) name.
fn named_uris(text: &str, language: Language) -> Vec<String> {
    match language {
        Language::Qvto => oxy_qvt::operational::syntax::parse(text)
            .0
            .modeltypes
            .iter()
            .flat_map(|m| m.packages.iter().filter_map(|p| p.uri.as_ref().map(|(u, _)| u.clone())))
            .collect(),
        Language::Qvtr => oxy_qvt::relational::syntax::parse(text).0.imports.into_iter().map(|i| i.uri).collect(),
        _ => Vec::new(),
    }
}

/// The root package of a class's package.
fn root_package(mm: &Metamodel, mut p: PackageId) -> PackageId {
    while let Some(s) = mm.package(p).super_package {
        p = s;
    }
    p
}

impl Workspace {
    /// The packages a URI of a QVT document stands for: an nsURI, a URI a metamodel file is
    /// known by (e.g. one of the package registry), or a metamodel file relative to the
    /// document (an `.ecore` file, or an open OCLinEcore document).
    fn qvt_packages(&self, document: &str, uri: &str) -> Vec<PackageId> {
        let mm = self.metamodel();
        let found = oxy_qvt::packages_of_uri(mm, uri);
        if !found.is_empty() {
            return found;
        }
        let base = uri.split('#').next().unwrap_or(uri);
        let path =
            self.locator().locate(base).unwrap_or_else(|| oxy_ecore::ecore_loader::resolve_relative(document, base));
        let canonical = oxy_ecore::ecore_loader::canonical_uri(Path::new(&path));
        for candidate in [&path, &canonical] {
            if let Some(r) = mm.resource_of_uri(candidate) {
                return mm.resources()[r].roots.clone();
            }
        }
        for name in self.document_names() {
            if (Path::new(name) == Path::new(&path) || Path::new(name) == Path::new(&canonical))
                && let Some(built) = self.oclinecore_document(name)
            {
                return built.packages.clone();
            }
        }
        Vec::new()
    }

    /// Loads the `.ecore` files a QVT document's model types or imports name and that are not
    /// loaded yet (problems show up as the document's diagnostics).
    pub(crate) fn load_qvt_imports(&mut self, name: &str, text: &str, language: Language) {
        for uri in named_uris(text, language) {
            if !self.qvt_packages(name, &uri).is_empty() {
                continue;
            }
            let base = uri.split('#').next().unwrap_or(&uri).to_string();
            let path =
                self.locator().locate(&base).unwrap_or_else(|| oxy_ecore::ecore_loader::resolve_relative(name, &base));
            let file = Path::new(&path);
            let oclinecore = file.extension().is_some_and(|e| e.eq_ignore_ascii_case("oclinecore"));
            if file.is_file() && !oclinecore {
                let _ = self.load_ecore_file(file);
            }
        }
    }

    /// Compiles an open QVT document.
    pub(crate) fn compile_qvt(&self, name: &str, ocl: &OclCompiled) -> Result<Compiled, Error> {
        let text =
            self.document_text(name).ok_or_else(|| Error::NotFound(format!("no open document named '{name}'")))?;
        let language = self
            .document_language(name)
            .and_then(qvt_language)
            .ok_or_else(|| Error::Invalid(format!("'{name}' is not a QVT document")))?;
        let lookup = |uri: &str| self.qvt_packages(name, uri);
        Ok(oxy_qvt::compile(language, self.metamodel(), &ocl.env, text, &lookup))
    }

    /// The diagnostics of an open QVT document.
    pub(crate) fn qvt_diagnostics(&self, name: &str, ocl: &OclCompiled) -> Vec<Diagnostic> {
        let Ok(compiled) = self.compile_qvt(name, ocl) else { return Vec::new() };
        compiled
            .diagnostics
            .iter()
            .map(|d| Diagnostic {
                severity: crate::workspace::severity(d.severity),
                message: d.message.clone(),
                source: name.to_string(),
                origin: Origin::Document { name: name.to_string() },
                range: self.document_range(name, &d.span.range()),
            })
            .collect()
    }

    /// The QVT transformations of the open documents. With `model` (a model resource), each
    /// says which of its parameters the model can be bound to.
    pub fn qvt_transformations(&self, model: Option<&str>) -> Result<Vec<QvtTransformationInfo>, Error> {
        let mm = self.metamodel();
        let packages: Option<Vec<PackageId>> = match model {
            Some(m) => {
                let resource = self.resource_index(m)?;
                let model = self.model();
                let mut used = Vec::new();
                for o in model.object_ids() {
                    if model.object(o).resource == resource && !model.is_deleted(o) {
                        let p = root_package(mm, mm.classifier(model.class_of(o)).package);
                        if !used.contains(&p) {
                            used.push(p);
                        }
                    }
                }
                Some(used)
            }
            None => None,
        };
        let ocl = self.compile(true, None);
        let mut out = Vec::new();
        for name in self.document_names() {
            let Some(language) = self.document_language(name).filter(|l| l.is_qvt()) else { continue };
            let compiled = self.compile_qvt(name, &ocl)?;
            let applicable = match &packages {
                Some(used) if !used.is_empty() => compiled
                    .parameters
                    .iter()
                    .filter(|p| p.is_input() && !p.packages.is_empty())
                    .filter(|p| {
                        let roots: Vec<PackageId> = p.packages.iter().map(|x| root_package(mm, *x)).collect();
                        used.iter().all(|u| roots.contains(u))
                    })
                    .map(|p| p.name.clone())
                    .collect(),
                _ => Vec::new(),
            };
            out.push(QvtTransformationInfo {
                document: name.to_string(),
                language,
                name: compiled.name.clone().unwrap_or_default(),
                parameters: compiled
                    .parameters
                    .iter()
                    .map(|p| QvtParameterInfo {
                        name: p.name.clone(),
                        direction: p.direction.map(|d| d.keyword().to_string()),
                        model_type: p.model_type.clone(),
                        metamodels: p.packages.iter().map(|x| mm.package(*x).ns_uri.clone()).collect(),
                    })
                    .collect(),
                targets: compiled.targets(),
                errors: compiled.diagnostics.iter().filter(|d| d.is_error()).count(),
                applicable,
            });
        }
        Ok(out)
    }

    /// Runs a QVT transformation on a copy of the workspace's models and returns the models
    /// it produced or changed (as XMI or HUTN text), what it logged, the problems it met and
    /// the invariants the results violate. The workspace is not changed.
    pub fn run_qvt(&self, request: &QvtRunRequest) -> Result<QvtRunResult, Error> {
        let name = request.document.as_str();
        let ocl = self.compile(true, None);
        let compiled = self.compile_qvt(name, &ocl)?;
        if let Some(e) = compiled.diagnostics.iter().find(|d| d.is_error()) {
            let range = self.document_range(name, &e.span.range());
            return Err(Error::Invalid(format!(
                "{name} has errors (line {}: {}); fix them first",
                range.start.line + 1,
                e.message
            )));
        }
        let mm = self.metamodel();
        let mut model: Model = self.model().clone();
        let relational = compiled.language == oxy_qvt::Language::Relational;
        let target = match (&request.target, compiled.targets().as_slice()) {
            (Some(t), _) => Some(t.clone()),
            (None, [only]) if relational => Some(only.clone()),
            _ => None,
        };
        let folder = Path::new(name).parent().map(Path::to_path_buf).unwrap_or_default();
        let mut bindings = Vec::new();
        let mut outputs: Vec<(String, usize, String)> = Vec::new();
        for p in &compiled.parameters {
            let given = request.models.iter().find(|b| b.parameter == p.name);
            let creates = match p.direction {
                Some(Direction::Out) => true,
                Some(_) => false,
                None => target.as_deref() == Some(&p.name),
            };
            let resource = match given {
                Some(b) => self.resource_index(&b.model)?,
                None if creates => {
                    let path = match request.outputs.iter().find(|o| o.parameter == p.name) {
                        Some(o) => o.path.clone(),
                        None => folder.join(format!("{}.xmi", p.name)).to_string_lossy().into_owned(),
                    };
                    model.add_resource(path)
                }
                None => return Err(Error::Invalid(format!("no model is given for the parameter '{}'", p.name))),
            };
            if creates || p.direction == Some(Direction::InOut) {
                let path = request
                    .outputs
                    .iter()
                    .find(|o| o.parameter == p.name)
                    .map(|o| o.path.clone())
                    .unwrap_or_else(|| model.resources()[resource].uri.clone());
                outputs.push((p.name.clone(), resource, path));
            }
            bindings.push(Binding { parameter: p.name.clone(), resource });
        }
        let options = RunOptions {
            target,
            check_only: request.check,
            configuration: request.configuration.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        };
        let result = oxy_qvt::run(&compiled, mm, &ocl.env, &mut model, &bindings, &options).map_err(Error::Invalid)?;

        let mut out = QvtRunResult {
            log: result.log,
            violations: result.violations,
            applications: result.applications,
            created: result.created,
            aborted: result.aborted,
            ..QvtRunResult::default()
        };
        out.problems = result
            .problems
            .iter()
            .map(|p| Diagnostic {
                severity: Severity::Error,
                message: p.message.clone(),
                source: name.to_string(),
                origin: Origin::Document { name: name.to_string() },
                range: p.span.map(|s| self.document_range(name, &s.range())).unwrap_or_default(),
            })
            .collect();
        if !request.check {
            let mut objects: Vec<ObjectId> = Vec::new();
            for (parameter, resource, path) in outputs {
                let hutn_format = match request.format.as_deref() {
                    Some(f) => f.eq_ignore_ascii_case("hutn"),
                    None => path.to_ascii_lowercase().ends_with(".hutn"),
                };
                let text = if hutn_format {
                    hutn::write(mm, &model, resource)
                } else {
                    xmi_writer::write_resource(mm, &model, resource, &self.schema_locations(&path))
                };
                let contents: Vec<ObjectId> = model.resources()[resource]
                    .roots
                    .iter()
                    .flat_map(|r| std::iter::once(*r).chain(model.all_contents(mm, *r)))
                    .collect();
                out.outputs.push(QvtOutput {
                    parameter,
                    path,
                    format: if hutn_format { "hutn" } else { "xmi" }.into(),
                    text,
                    objects: contents.len(),
                });
                objects.extend(contents);
            }
            out.invariants = oxy_ocl::validate::validate_objects(mm, &model, &ocl.env, &objects)
                .into_iter()
                .filter(|r| r.status != oxy_ocl::Status::Satisfied)
                .map(|r| {
                    let inv = &ocl.env.invariants()[r.invariant];
                    let what =
                        format!("{}::{}", mm.classifier(inv.context).name, inv.name.as_deref().unwrap_or("invariant"));
                    let on = model.label(mm, r.object);
                    match (&r.status, &r.message) {
                        (oxy_ocl::Status::Error(e), _) => format!("{what} on {on}: {e}"),
                        (_, Some(m)) => format!("{what} is violated on {on}: {m}"),
                        _ => format!("{what} is violated on {on}"),
                    }
                })
                .collect();
        }
        Ok(out)
    }
}
