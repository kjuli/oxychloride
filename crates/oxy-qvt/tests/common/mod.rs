//! Loading the examples of `examples/qvt` and running transformations on them.

#![allow(dead_code)]

use std::path::PathBuf;

use oxy_ecore::hutn;
use oxy_ecore::{Metamodel, Model};
use oxy_ocl::oclinecore;
use oxy_ocl::syntax::parse_ecore_document;
use oxy_ocl::{Environment, EnvironmentBuilder};
use oxy_qvt::{Binding, Compiled, Language, RunOptions, RunResult};

pub fn example(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/qvt").join(name)
}

pub fn read(name: &str) -> String {
    std::fs::read_to_string(example(name)).unwrap()
}

/// The metamodels of the example (with their OCL), and its model.
pub struct Fixture {
    pub mm: Metamodel,
    pub env: Environment,
    pub model: Model,
    pub source: usize,
}

pub fn fixture() -> Fixture {
    let mut mm = Metamodel::new();
    let mut built = Vec::new();
    for f in ["uml.oclinecore", "rdbms.oclinecore"] {
        let text = read(f);
        let (doc, diags) = parse_ecore_document(&text);
        let b = oclinecore::build(&mut mm, f, &text, doc, diags);
        assert!(b.diagnostics.is_empty(), "{f}: {:?}", b.diagnostics);
        built.push((f, text, b));
    }
    let env = {
        let mut builder = EnvironmentBuilder::new(&mm);
        for (f, text, b) in &built {
            builder.add_oclinecore(f, text, b);
        }
        builder.build()
    };
    assert!(!env.has_errors(), "{:?}", env.diagnostics());
    let mut model = Model::new();
    let load = hutn::load(&read("company.hutn"), "company.hutn", &mm, &mut model);
    assert!(load.problems.is_empty(), "{:?}", load.problems);
    Fixture { mm, env, model, source: load.resource }
}

impl Fixture {
    pub fn compile(&self, language: Language, text: &str) -> Compiled {
        let mm = &self.mm;
        oxy_qvt::compile(language, mm, &self.env, text, &|uri| oxy_qvt::packages_of_uri(mm, uri))
    }

    /// Runs a transformation from `source` into a new resource bound to `target`.
    pub fn run(&mut self, compiled: &Compiled, options: &RunOptions) -> (RunResult, usize) {
        let out = self.model.add_resource("out.xmi");
        let bindings = vec![
            Binding { parameter: "source".into(), resource: self.source },
            Binding { parameter: "target".into(), resource: out },
        ];
        let result = oxy_qvt::run(compiled, &self.mm, &self.env, &mut self.model, &bindings, options).unwrap();
        (result, out)
    }

    pub fn hutn(&self, resource: usize) -> String {
        hutn::write(&self.mm, &self.model, resource)
    }
}

pub fn no_errors(c: &Compiled) {
    let errors: Vec<_> = c.diagnostics.iter().filter(|d| d.is_error()).collect();
    assert!(errors.is_empty(), "{errors:#?}");
}
