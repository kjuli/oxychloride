//! Ecore metamodels and XMI instance models for the Oxychloride OCL engine.
//!
//! * [`Metamodel`] holds any number of loaded `EPackage`s (plus Ecore itself) and answers the
//!   questions an OCL engine needs: name lookup, inheritance, features and operations.
//! * [`Model`] holds dynamic instances of those classes, loaded from XMI or built in code.

mod builtin;
pub mod ecore_loader;
pub mod ecore_writer;
mod error;
pub mod hutn;
pub mod metamodel;
pub mod model;
pub mod xmi;
pub mod xmi_writer;
pub mod xml;
pub mod xsd_writer;

pub use ecore_loader::EcoreLoad;
pub use error::{LoadError, Problem, ProblemSeverity};
pub use metamodel::*;
pub use model::{Model, ModelResource, ModelValue, Object, ObjectId, UnresolvedReference, default_value, parse_value};
pub use xmi::{XmiDocument, XmiLoad};

/// Annotation sources under which EMF/Eclipse OCL store OCL expressions in `.ecore` files.
pub const OCL_ANNOTATION_SOURCES: &[&str] = &[
    "http://www.eclipse.org/emf/2002/Ecore/OCL/Pivot",
    "http://www.eclipse.org/emf/2002/Ecore/OCL",
    "http://www.eclipse.org/emf/2002/Ecore/OCL/LPG",
    "http://www.eclipse.org/ocl/examples/OCL",
];

/// The Ecore annotation source; its `constraints` detail lists a class's invariant names.
pub const ECORE_ANNOTATION_SOURCE: &str = "http://www.eclipse.org/emf/2002/Ecore";

impl Metamodel {
    /// Loads an `.ecore` file (and any `.ecore` files it references).
    pub fn load_ecore_file(&mut self, path: impl AsRef<std::path::Path>) -> Result<EcoreLoad, LoadError> {
        ecore_loader::load_file(self, path)
    }

    /// Loads `.ecore` content from a string; `uri` identifies the resource.
    pub fn load_ecore_str(&mut self, text: &str, uri: &str) -> Result<EcoreLoad, LoadError> {
        ecore_loader::load_str(self, text, uri)
    }

    /// Serializes a loaded resource as an `.ecore` document.
    pub fn write_ecore(&self, resource: usize) -> String {
        ecore_writer::write_resource(self, resource)
    }
}
