//! Code generation (see the `oxy-codegen` crate): native code for the workspace's
//! metamodels, with their OCL translated.

use oxy_ecore::PackageId;

use crate::protocol::{CodeFile, CodeProblem, CodeRequest, CodeTarget, GeneratedCode, Severity};
use crate::workspace::{Error, Workspace};

/// The languages code can be generated for.
pub fn code_targets() -> Vec<CodeTarget> {
    oxy_codegen::TARGETS
        .iter()
        .map(|t| CodeTarget { id: t.id().into(), name: t.name().into(), description: t.description().into() })
        .collect()
}

impl Workspace {
    /// Generates code for the packages of a metamodel resource (or the named packages, or
    /// all loaded metamodels), with the OCL embedded in the metamodels and that of the
    /// Complete OCL documents. The files are returned, not written.
    pub fn generate_code(&self, request: &CodeRequest) -> Result<GeneratedCode, Error> {
        let mm = &self.mm;
        let packages: Vec<PackageId> = match (&request.resource, &request.packages) {
            (Some(resource), _) => {
                let index = mm
                    .resource_of_uri(resource)
                    .or_else(|| {
                        let canonical = oxy_ecore::ecore_loader::canonical_uri(std::path::Path::new(resource));
                        mm.resource_of_uri(&canonical)
                    })
                    .ok_or_else(|| Error::NotFound(format!("no metamodel '{resource}' is loaded")))?;
                mm.resources()[index].roots.clone()
            }
            (None, Some(names)) => {
                let mut out = Vec::new();
                for name in names {
                    let path: Vec<&str> = name.split("::").collect();
                    match mm.find_packages_qualified(&path).first() {
                        Some(p) => out.push(*p),
                        None => return Err(Error::NotFound(format!("no package '{name}' is loaded"))),
                    }
                }
                out
            }
            (None, None) => mm.user_root_packages().collect(),
        };
        if packages.is_empty() {
            return Err(Error::NotFound("the metamodel has no packages (does it have errors?)".into()));
        }
        let target = oxy_codegen::target(&request.target).ok_or_else(|| {
            let known: Vec<&str> = oxy_codegen::TARGETS.iter().map(|t| t.id()).collect();
            Error::Invalid(format!("unknown target '{}' (known: {})", request.target, known.join(", ")))
        })?;
        let compiled = self.compile(true, request.documents.as_deref());
        let options = oxy_codegen::Options { name: request.name.clone() };
        let (model, problems) = oxy_codegen::build(mm, &compiled.env, &packages, options.name.as_deref());
        let mut output = oxy_codegen::Output { files: Vec::new(), problems };
        target.generate(&model, &options, &mut output);
        Ok(GeneratedCode {
            target: request.target.clone(),
            packages: model.packages.iter().map(|p| p.path.join("::")).collect(),
            files: output.files.into_iter().map(|f| CodeFile { path: f.path, text: f.text }).collect(),
            problems: output
                .problems
                .into_iter()
                .map(|p| CodeProblem {
                    severity: match p.severity {
                        oxy_codegen::Severity::Error => Severity::Error,
                        oxy_codegen::Severity::Warning => Severity::Warning,
                    },
                    message: p.message,
                    origin: p.origin,
                })
                .collect(),
        })
    }
}
