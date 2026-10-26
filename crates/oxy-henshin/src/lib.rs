//! Henshin graph transformations over Ecore models, written in Henshin's textual syntax.
//!
//! * [`syntax`] parses `.henshin_text` documents (the Xtext grammar of
//!   `org.eclipse.emf.henshin.text`);
//! * [`Module`] resolves their rules and units against a [`Metamodel`](oxy_ecore::Metamodel),
//!   reporting problems with ranges;
//! * [`Graph`] holds the objects of a model as an in-place transformable graph;
//! * [`Engine`] matches and applies rules (application conditions, attribute conditions,
//!   multi-rules) and executes units;
//! * [`explore`] computes the state space of a set of rules, merging isomorphic states.
//!
//! ```
//! use oxy_ecore::{Metamodel, Model};
//! use oxy_henshin::{Engine, Graph, Module};
//!
//! let mut mm = Metamodel::new();
//! mm.load_ecore_str(r##"<?xml version="1.0" encoding="UTF-8"?>
//! <ecore:EPackage xmi:version="2.0" xmlns:xmi="http://www.omg.org/XMI"
//!     xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
//!     xmlns:ecore="http://www.eclipse.org/emf/2002/Ecore" name="net" nsURI="http://net" nsPrefix="net">
//!   <eClassifiers xsi:type="ecore:EClass" name="Net">
//!     <eStructuralFeatures xsi:type="ecore:EReference" name="places" upperBound="-1" eType="#//Place" containment="true"/>
//!   </eClassifiers>
//!   <eClassifiers xsi:type="ecore:EClass" name="Place">
//!     <eStructuralFeatures xsi:type="ecore:EAttribute" name="tokens"
//!         eType="ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//EInt"/>
//!   </eClassifiers>
//! </ecore:EPackage>"##, "net.ecore").unwrap();
//! let module = Module::compile(&mm, "ePackageImport net
//!     rule addPlace() { graph { node n:Net  create node p:Place { tokens=1 }  edges [(create n->p:places)] } }");
//! assert!(module.problems.is_empty(), "{:?}", module.problems);
//!
//! let mut graph = Graph::new();
//! graph.add_node(mm.find_classifiers("Net")[0]);
//! let engine = Engine::new(&mm, &module);
//! let m = engine.matches(&graph, 0, &[], 10).pop().unwrap();
//! engine.apply_match(&mut graph, 0, &m).unwrap();
//! assert_eq!(graph.len(), 2);
//! ```

pub mod engine;
pub mod explore;
pub mod graph;
pub mod js;
pub mod module;
pub mod print;
pub mod syntax;
pub mod value;
pub mod xmi;

pub use engine::{Application, Engine, Execution, Match};
pub use explore::{ExploreOptions, Selected, StateSpace, explore};
pub use graph::{Graph, NodeId};
pub use module::{Callable, Module, Param, ParamType, Rule, Unit};
pub use value::Value;
