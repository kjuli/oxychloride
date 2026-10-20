//! `oxy` — check, run and evaluate OCL against Ecore metamodels and models.

mod render;

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use oxychloride::{Diagnostic, Language, ObjectRef, Severity, ValidateOptions, Workspace};

use render::Renderer;

#[derive(Parser)]
#[command(name = "oxy", version, about = "OCL for Ecore models", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
    /// Output format.
    #[arg(long, global = true, value_enum, default_value_t = Format::Text)]
    format: Format,
    /// Disable colored output (also honors NO_COLOR).
    #[arg(long, global = true)]
    no_color: bool,
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum Format {
    Text,
    Json,
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum ConvertTo {
    /// An OCLinEcore document (from an `.ecore` file).
    Oclinecore,
    /// An `.ecore` file (from an OCLinEcore document).
    Ecore,
    /// A HUTN document (from an XMI model).
    Hutn,
    /// An XMI model (from a HUTN document).
    Xmi,
}

#[derive(clap::Args)]
struct Inputs {
    /// Metamodels to load: `.ecore` files or OCLinEcore documents (`.oclinecore`). Imports in
    /// documents and schema locations in models are loaded automatically.
    #[arg(short, long = "ecore", value_name = "FILE")]
    ecore: Vec<PathBuf>,
    /// Instance models to load: XMI files or HUTN documents (`.hutn`). Model files they
    /// reference are loaded too.
    #[arg(short, long = "model", value_name = "FILE")]
    model: Vec<PathBuf>,
    /// Map a URI prefix to a directory, e.g. `pathmap://PCM_MODELS/=path/to/models/`.
    #[arg(long = "map", value_name = "PREFIX=DIR")]
    map: Vec<String>,
}

#[derive(Subcommand)]
enum Command {
    /// Parse and check documents: Complete OCL, OCLinEcore metamodels and HUTN models (for
    /// models, the constraints their objects violate are reported too).
    Check {
        /// Documents (`.ocl`, `.oclinecore`, `.hutn`).
        #[arg(required = true)]
        documents: Vec<PathBuf>,
        #[command(flatten)]
        inputs: Inputs,
    },
    /// Validate models: evaluate the invariants of OCL documents and of the metamodels, and
    /// check the pre- and postconditions of operations.
    Run {
        /// Documents: Complete OCL (`.ocl`), and metamodels (`.oclinecore`) or models (`.hutn`)
        /// as text.
        documents: Vec<PathBuf>,
        #[command(flatten)]
        inputs: Inputs,
        /// Skip the constraints embedded in the metamodels.
        #[arg(long)]
        no_ecore: bool,
        /// Skip checking pre- and postconditions.
        #[arg(long)]
        no_contracts: bool,
        /// Also list satisfied constraints.
        #[arg(long)]
        all: bool,
    },
    /// Check the OCL embedded in metamodels (and validate models against it).
    Ecore {
        /// Metamodels whose embedded OCL to check (`.ecore` or `.oclinecore`).
        #[arg(required = true)]
        metamodels: Vec<PathBuf>,
        /// Instance models to validate against the embedded invariants.
        #[arg(short, long = "model", value_name = "FILE")]
        model: Vec<PathBuf>,
        /// Also list satisfied constraints.
        #[arg(long)]
        all: bool,
    },
    /// Evaluate an OCL expression.
    Eval {
        /// The expression.
        expression: String,
        #[command(flatten)]
        inputs: Inputs,
        /// Documents whose definitions should be available.
        #[arg(short, long = "document", value_name = "FILE")]
        document: Vec<PathBuf>,
        /// The object `self` refers to: a URI fragment like `//@books.0`, optionally
        /// prefixed with the model file (`library.xmi#//@books.0`).
        #[arg(short, long)]
        context: Option<String>,
    },
    /// Print the structure of metamodels.
    Info {
        #[arg(required = true)]
        metamodels: Vec<PathBuf>,
    },
    /// Convert between the XML and the textual formats: `.ecore` files and OCLinEcore
    /// documents, XMI models and HUTN documents.
    Convert {
        /// The file to convert.
        input: PathBuf,
        /// Where to write the result (default: standard output).
        #[arg(short, long, value_name = "FILE")]
        output: Option<PathBuf>,
        /// The target format (default: from the output's extension, else the counterpart of
        /// the input's format).
        #[arg(long, value_enum)]
        to: Option<ConvertTo>,
        #[command(flatten)]
        inputs: Inputs,
    },
    /// Serve the JSON-RPC protocol on stdin/stdout (one JSON message per line).
    Serve,
    /// Run the language server (LSP on stdin/stdout), e.g. for the VS Code extension.
    Lsp {
        /// Accepted for compatibility with LSP clients; stdin/stdout is the only transport.
        #[arg(long)]
        stdio: bool,
    },
}

/// Exit codes: 0 success, 1 problems found (errors, violations), 2 the command failed.
fn main() -> ExitCode {
    let cli = Cli::parse();
    let color =
        !cli.no_color && std::env::var_os("NO_COLOR").is_none() && std::io::IsTerminal::is_terminal(&std::io::stdout());
    let r = Renderer::new(color);
    match run(cli.command, cli.format, &r) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(message) => {
            eprintln!("{}: {message}", r.error("error"));
            ExitCode::from(2)
        }
    }
}

/// Loads metamodels and models. OCLinEcore and HUTN files are opened as documents (OCLinEcore
/// first, so that models can use their classes); their names are returned.
fn load(ws: &mut Workspace, inputs: &Inputs, warnings: &mut Vec<Diagnostic>) -> Result<Vec<String>, String> {
    for m in &inputs.map {
        let (from, to) = m.split_once('=').ok_or_else(|| format!("invalid --map '{m}' (expected PREFIX=DIR)"))?;
        ws.map_uri(from, to);
    }
    let mut documents = Vec::new();
    for e in &inputs.ecore {
        if Language::from_path(&e.to_string_lossy()) == Language::OclInEcore {
            documents.extend(open_documents(ws, std::slice::from_ref(e))?);
        } else {
            let r = ws.load_ecore_file(e).map_err(|e| e.to_string())?;
            warnings.extend(r.warnings);
        }
    }
    for m in &inputs.model {
        if Language::from_path(&m.to_string_lossy()) == Language::Hutn {
            documents.extend(open_documents(ws, std::slice::from_ref(m))?);
        } else {
            let r = ws.load_model_file(m).map_err(|e| e.to_string())?;
            warnings.extend(r.warnings);
        }
    }
    Ok(documents)
}

/// Opens documents (OCLinEcore documents first: the others may depend on them).
fn open_documents(ws: &mut Workspace, documents: &[PathBuf]) -> Result<Vec<String>, String> {
    let mut sorted: Vec<&PathBuf> = documents.iter().collect();
    sorted.sort_by_key(|d| Language::from_path(&d.to_string_lossy()) != Language::OclInEcore);
    let mut names = Vec::new();
    for d in sorted {
        let text = std::fs::read_to_string(d).map_err(|e| format!("cannot read {}: {e}", d.display()))?;
        let name = d.to_string_lossy().into_owned();
        ws.open_document(&name, &text);
        names.push(name);
    }
    Ok(names)
}

/// The problems loading the HUTN documents among `names`.
fn model_problems(ws: &Workspace, names: &[String]) -> Vec<Diagnostic> {
    names.iter().flat_map(|n| ws.model_problems(n)).collect()
}

fn has_models(ws: &Workspace) -> bool {
    !ws.model().is_empty()
}

fn print_json(value: &impl serde::Serialize) {
    println!("{}", serde_json::to_string_pretty(value).expect("serializable"));
}

fn has_errors(diags: &[Diagnostic]) -> bool {
    diags.iter().any(|d| d.severity == Severity::Error)
}

fn run(command: Command, format: Format, r: &Renderer) -> Result<bool, String> {
    let mut ws = Workspace::new();
    let mut warnings = Vec::new();
    match command {
        Command::Check { documents, inputs } => {
            load(&mut ws, &inputs, &mut warnings)?;
            let names = open_documents(&mut ws, &documents)?;
            let mut diags = warnings;
            for n in &names {
                diags.extend(ws.diagnostics(Some(n)));
            }
            if format == Format::Json {
                print_json(&serde_json::json!({ "diagnostics": diags }));
            } else {
                let sources = sources(&ws);
                for d in &diags {
                    print!("{}", r.diagnostic(d, sources.get(&d.source).map(String::as_str)));
                }
                println!("{}", r.diagnostic_summary(&diags, names.len()));
            }
            Ok(!has_errors(&diags))
        }
        Command::Run { documents, inputs, no_ecore, no_contracts, all } => {
            let mut opened = load(&mut ws, &inputs, &mut warnings)?;
            let names = open_documents(&mut ws, &documents)?;
            opened.extend(names.iter().cloned());
            if !has_models(&ws) {
                return Err("no model given (use --model FILE, or a .hutn document)".into());
            }
            warnings.extend(model_problems(&ws, &opened));
            let options = ValidateOptions {
                ecore: !no_ecore,
                documents: Some(names),
                objects: None,
                include_satisfied: all,
                contracts: !no_contracts,
            };
            let report = ws.validate(&options).map_err(|e| e.to_string())?;
            if format == Format::Json {
                print_json(&report);
            } else {
                let sources = sources(&ws);
                for d in warnings.iter().chain(&report.diagnostics) {
                    print!("{}", r.diagnostic(d, sources.get(&d.source).map(String::as_str)));
                }
                print!("{}", r.report(&report));
            }
            Ok(report.is_ok() && !has_errors(&warnings))
        }
        Command::Ecore { metamodels, model, all } => {
            let inputs = Inputs { ecore: metamodels, model, map: Vec::new() };
            let opened = load(&mut ws, &inputs, &mut warnings)?;
            warnings.extend(model_problems(&ws, &opened));
            let model = inputs.model;
            let mut diags = warnings;
            // The constraint violations of HUTN models are part of the validation report.
            let is_model = |d: &Diagnostic| matches!(&d.origin, oxychloride::Origin::Document { name } if Language::from_path(name) == Language::Hutn);
            diags.extend(ws.diagnostics(None).into_iter().filter(|d| !is_model(d)));
            let report = if model.is_empty() {
                None
            } else {
                let options = ValidateOptions { include_satisfied: all, ..Default::default() };
                Some(ws.validate(&options).map_err(|e| e.to_string())?)
            };
            if format == Format::Json {
                print_json(&serde_json::json!({ "diagnostics": diags, "validation": report }));
            } else {
                let sources = sources(&ws);
                for d in &diags {
                    print!("{}", r.diagnostic(d, sources.get(&d.source).map(String::as_str)));
                }
                let count = ws.packages().iter().map(count_invariants).sum::<usize>();
                println!("{}", r.ecore_summary(&diags, count));
                if let Some(report) = &report {
                    print!("{}", r.report(report));
                }
            }
            Ok(!has_errors(&diags) && report.is_none_or(|r| r.is_ok()))
        }
        Command::Eval { expression, inputs, document, context } => {
            load(&mut ws, &inputs, &mut warnings)?;
            open_documents(&mut ws, &document)?;
            let context = context.map(|c| match c.split_once('#') {
                Some((resource, fragment)) => ObjectRef { resource: Some(resource.into()), fragment: fragment.into() },
                None => ObjectRef { resource: None, fragment: c },
            });
            let result = ws.evaluate(&expression, context.as_ref()).map_err(|e| e.to_string())?;
            if format == Format::Json {
                print_json(&result);
            } else {
                for d in &result.diagnostics {
                    eprint!("{}", r.diagnostic(d, Some(&expression)));
                }
                println!("{}", result.display);
                if !result.invalid {
                    eprintln!("{}", r.dim(&format!(": {}", result.ty)));
                }
                for c in &result.contracts {
                    eprintln!("{}", r.contract(c));
                }
            }
            Ok(!result.invalid && !has_errors(&result.diagnostics) && result.contracts.is_empty())
        }
        Command::Info { metamodels } => {
            let inputs = Inputs { ecore: metamodels, model: Vec::new(), map: Vec::new() };
            load(&mut ws, &inputs, &mut warnings)?;
            let packages = ws.packages();
            if format == Format::Json {
                print_json(&packages);
            } else {
                for p in &packages {
                    print!("{}", r.package(p, 0));
                }
            }
            Ok(true)
        }
        Command::Convert { input, output, to, inputs } => {
            let opened = load(&mut ws, &inputs, &mut warnings)?;
            let input_name = input.to_string_lossy().into_owned();
            let input_language = Language::from_path(&input_name);
            let output_ext = output.as_ref().and_then(|o| o.extension()).map(|e| e.to_string_lossy().to_lowercase());
            let to = to.unwrap_or_else(|| match (output_ext.as_deref(), input_language) {
                (Some("oclinecore"), _) => ConvertTo::Oclinecore,
                (Some("ecore"), _) => ConvertTo::Ecore,
                (Some("hutn"), _) => ConvertTo::Hutn,
                (Some("xmi"), _) => ConvertTo::Xmi,
                (_, Language::OclInEcore) => ConvertTo::Ecore,
                (_, Language::Hutn) => ConvertTo::Xmi,
                _ if input_name.ends_with(".ecore") => ConvertTo::Oclinecore,
                _ => ConvertTo::Hutn,
            });
            let text = match to {
                ConvertTo::Oclinecore => {
                    ws.convert_file(&input_name, Language::OclInEcore).map_err(|e| e.to_string())?
                }
                ConvertTo::Hutn => ws.convert_file(&input_name, Language::Hutn).map_err(|e| e.to_string())?,
                ConvertTo::Ecore | ConvertTo::Xmi => {
                    let expected = if to == ConvertTo::Ecore { Language::OclInEcore } else { Language::Hutn };
                    if input_language != expected {
                        return Err(format!("{input_name} is not a .{} document", expected.extension()));
                    }
                    let name = open_documents(&mut ws, std::slice::from_ref(&input))?.remove(0);
                    let mut problems =
                        if expected == Language::Hutn { ws.model_problems(&name) } else { ws.diagnostics(Some(&name)) };
                    problems.extend(opened.iter().flat_map(|n| ws.diagnostics(Some(n))));
                    let sources = sources(&ws);
                    for d in warnings.iter().chain(&problems) {
                        eprint!("{}", r.diagnostic(d, sources.get(&d.source).map(String::as_str)));
                    }
                    if has_errors(&problems) {
                        return Ok(false);
                    }
                    let target = output.as_ref().map(|o| o.to_string_lossy().into_owned());
                    ws.export_document(&name, target.as_deref()).map_err(|e| e.to_string())?.text
                }
            };
            match output {
                Some(path) => {
                    std::fs::write(&path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))?
                }
                None => print!("{text}"),
            }
            Ok(true)
        }
        Command::Lsp { .. } => {
            oxy_lsp::run_stdio().map_err(|e| e.to_string())?;
            Ok(true)
        }
        Command::Serve => {
            let mut server = oxychloride::rpc::Server::new();
            let stdin = std::io::stdin();
            let mut stdout = std::io::stdout().lock();
            for line in stdin.lock().lines() {
                let line = line.map_err(|e| e.to_string())?;
                if line.trim().is_empty() {
                    continue;
                }
                if let Some(response) = server.handle_str(&line) {
                    writeln!(stdout, "{response}").map_err(|e| e.to_string())?;
                    stdout.flush().map_err(|e| e.to_string())?;
                }
            }
            Ok(true)
        }
    }
}

fn count_invariants(p: &oxychloride::PackageInfo) -> usize {
    p.classifiers.iter().map(|c| c.invariants.len()).sum::<usize>()
        + p.subpackages.iter().map(count_invariants).sum::<usize>()
}

/// Texts of all OCL sources and documents, by name (for code snippets in diagnostics).
fn sources(ws: &Workspace) -> HashMap<String, String> {
    let compiled = ws.compile(true, None);
    let mut out: HashMap<String, String> =
        compiled.env.sources().map(|(_, s)| (s.name.clone(), s.text.clone())).collect();
    for name in ws.document_names() {
        out.insert(name.to_string(), ws.document_text(name).unwrap_or_default().to_string());
    }
    out
}
