//! Oxychloride: an OCL engine for Ecore models.
//!
//! This crate is the integration surface of the engine:
//!
//! * [`Workspace`] — load `.ecore` metamodels, XMI models and Complete OCL documents; check,
//!   validate and evaluate. Use this from Rust.
//! * [`rpc::Server`] — the same functionality as JSON-RPC 2.0, used by `oxy serve`, the
//!   C ABI (`oxy-ffi`) and the language server (`oxy-lsp`).
//! * [`protocol`] — the serializable request/response types.
//!
//! The lower layers are re-exported as [`ecore`] (metamodels and models), [`ocl`]
//! (parser, type checker, evaluator), [`henshin`] (graph transformations), [`qvt`] (QVT
//! model transformations) and [`codegen`] (code generation).
//!
//! ```
//! use oxychloride::{Workspace, ValidateOptions};
//!
//! let mut ws = Workspace::new();
//! ws.load_ecore_str(r#"<?xml version="1.0" encoding="UTF-8"?>
//! <ecore:EPackage xmi:version="2.0" xmlns:xmi="http://www.omg.org/XMI"
//!     xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
//!     xmlns:ecore="http://www.eclipse.org/emf/2002/Ecore" name="shop" nsURI="http://shop" nsPrefix="shop">
//!   <eClassifiers xsi:type="ecore:EClass" name="Item">
//!     <eStructuralFeatures xsi:type="ecore:EAttribute" name="price"
//!         eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EInt"/>
//!   </eClassifiers>
//! </ecore:EPackage>"#, "shop.ecore").unwrap();
//! ws.load_model_str(r#"<shop:Item xmlns:shop="http://shop" price="-3"/>"#, "items.xmi").unwrap();
//! ws.open_document("rules.ocl", "context shop::Item inv PositivePrice: price > 0");
//!
//! let report = ws.validate(&ValidateOptions::default()).unwrap();
//! assert_eq!(report.summary.violated, 1);
//!
//! let result = ws.evaluate("shop::Item.allInstances().price", None).unwrap();
//! assert_eq!(result.display, "Bag{-3}");
//! ```

mod diagram;
mod editor;
mod generate;
mod henshin_documents;
pub mod protocol;
mod qvt_documents;
mod registry;
pub mod rpc;
mod textdiff;
mod transform;
mod workspace;

pub use generate::code_targets;
pub use henshin_documents::counterpart;
pub use oxy_codegen as codegen;
pub use oxy_ecore as ecore;
pub use oxy_henshin as henshin;
pub use oxy_ocl as ocl;
pub use oxy_qvt as qvt;
pub use protocol::*;
pub use transform::ExploreRequest;
pub use workspace::{Compiled, Error, ValidateOptions, Workspace};
