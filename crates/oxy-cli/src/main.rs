//! `oxy` — check, run and evaluate OCL against Ecore metamodels and models.

mod render;

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use oxychloride::{
    Diagnostic, Language, MetamodelFormat, NewProjectRequest, ObjectRef, Project, RegistryEntry, Severity,
    ValidateOptions, Workspace,
};

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
    /// A Henshin model as Henshin's editors write it (`.henshin`, from a Henshin text).
    Henshin,
    /// A Henshin text (`.henshin_text`, from a `.henshin` file).
    HenshinText,
}

/// The documents of a new project besides its metamodel.
#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum Part {
    /// Constraints in Complete OCL (`.ocl`).
    Ocl,
    /// A model in HUTN (`.hutn`).
    Hutn,
    /// A model in XMI (`.xmi`).
    Xmi,
    /// A Henshin transformation as text (`.henshin_text`).
    Henshin,
    /// A Henshin transformation as Henshin's editors write it (`.henshin`).
    HenshinModel,
    /// A QVT Operational Mappings transformation (`.qvto`).
    Qvto,
    /// A QVT Relations transformation (`.qvtr`).
    Qvtr,
}

impl Part {
    fn language(self) -> Language {
        match self {
            Part::Ocl => Language::Ocl,
            Part::Hutn => Language::Hutn,
            Part::Xmi => Language::Xmi,
            Part::Henshin => Language::Henshin,
            Part::HenshinModel => Language::HenshinModel,
            Part::Qvto => Language::Qvto,
            Part::Qvtr => Language::Qvtr,
        }
    }
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
    /// Map a URI prefix to a directory (`pathmap://PCM_MODELS/=path/to/models/`), or register a
    /// URI for a metamodel file (`http://example.org/shop=shop.ecore`, see the package registry).
    #[arg(long = "map", value_name = "URI=PATH")]
    map: Vec<String>,
}

#[derive(Subcommand)]
enum Command {
    /// Parse and check documents: Complete OCL, OCLinEcore metamodels, HUTN models (for
    /// models, the constraints their objects violate are reported too), Henshin
    /// transformations (as text or as `.henshin` files) and QVT transformations. Without
    /// documents, all documents of the project (`oxychloride.json`) the current directory is
    /// in, with its package registry: models (HUTN and XMI) are checked against all
    /// constraints.
    Check {
        /// Documents (`.ocl`, `.oclinecore`, `.hutn`, `.xmi`, `.henshin_text`, `.henshin`,
        /// `.qvto`, `.qvtr`).
        documents: Vec<PathBuf>,
        /// Check the project in this folder (or with this manifest) instead.
        #[arg(long, value_name = "DIR", conflicts_with = "documents")]
        project: Option<PathBuf>,
        #[command(flatten)]
        inputs: Inputs,
    },
    /// Create a modeling project: a manifest (`oxychloride.json`), a metamodel, and
    /// constraints, a model and transformations of it to start from (all free of errors).
    New {
        /// The folder of the project (created if needed).
        dir: PathBuf,
        /// The project's name; its metamodel's package is named after it (default: the
        /// folder's name).
        #[arg(long)]
        name: Option<String>,
        /// A sentence about the project (for the manifest and the README).
        #[arg(long)]
        description: Option<String>,
        /// The nsURI of the metamodel's package (default: `http://example.org/<package>`).
        #[arg(long, value_name = "URI")]
        ns_uri: Option<String>,
        /// Write the metamodel as an `.ecore` file (registered in the project's package
        /// registry) instead of an OCLinEcore document.
        #[arg(long)]
        ecore: bool,
        /// The documents to create besides the metamodel (default: all of them).
        #[arg(long = "with", value_enum, value_delimiter = ',', value_name = "KIND", conflicts_with = "bare")]
        with: Vec<Part>,
        /// Only the manifest and the metamodel.
        #[arg(long)]
        bare: bool,
        /// The code generation target of the manifest (`oxy generate`, and VS Code's
        /// *Generate Code…*, use it for the project).
        #[arg(long, value_parser = targets())]
        codegen: Option<String>,
        /// Replace files that exist.
        #[arg(long)]
        force: bool,
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
    /// documents, XMI models and HUTN documents, Henshin's `.henshin` files and Henshin texts
    /// (`.henshin_text`; pass the metamodels with `-e`).
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
    /// Generate code from metamodels: classes with consistent references, XMI loading and
    /// saving, and their OCL (invariants, operation bodies, pre- and postconditions, derived
    /// features, definitions) translated into native code.
    Generate {
        /// Metamodels to generate code for (`.ecore` files or OCLinEcore documents). The
        /// packages they depend on are generated too.
        #[arg(required = true)]
        metamodels: Vec<PathBuf>,
        /// The target language.
        #[arg(short, long, value_parser = targets())]
        target: String,
        /// The directory to write the code into (created if needed).
        #[arg(short, long, value_name = "DIR")]
        out: PathBuf,
        /// Complete OCL documents with further constraints and definitions.
        #[arg(short, long = "document", value_name = "FILE")]
        document: Vec<PathBuf>,
        /// The name of the generated package or crate (default: the first package's name).
        #[arg(long)]
        name: Option<String>,
        /// Map a URI prefix to a directory (`pathmap://PCM_MODELS/=path/to/models/`), or register
        /// a URI for a metamodel file (`http://example.org/shop=shop.ecore`).
        #[arg(long = "map", value_name = "URI=PATH")]
        map: Vec<String>,
    },
    /// Run a QVT transformation — Operational Mappings (`.qvto`) or Relations (`.qvtr`) — on
    /// models, and write the models it produces or changes (the metamodels its model types or
    /// imports name are given with `-e` or found through `--map`).
    Qvt {
        /// The transformation (`.qvto` or `.qvtr`).
        transformation: PathBuf,
        /// The model of a parameter: `--in source=model.xmi` (or a `.hutn` document). Without
        /// it, a single input parameter takes the model given with `-m`.
        #[arg(long = "in", value_name = "PARAM=FILE")]
        bind: Vec<String>,
        /// Where to write the model of an output parameter: `--out target=schema.xmi` (a
        /// `.hutn` file gets HUTN). Without it, the output models are printed.
        #[arg(long = "out", value_name = "PARAM=FILE")]
        out: Vec<String>,
        /// QVTr: the parameter to run the transformation in the direction of (needed when its
        /// relations enforce several).
        #[arg(long)]
        target: Option<String>,
        /// QVTr: only check the target model (given with `--in`) against the relations.
        #[arg(long)]
        check: bool,
        /// QVTo: the value of a configuration property, in OCL syntax: `--config prefix="'T_'"`.
        #[arg(long = "config", value_name = "NAME=VALUE")]
        config: Vec<String>,
        /// Print the output models as HUTN (instead of XMI).
        #[arg(long)]
        hutn: bool,
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

/// The identifiers of the code generation targets.
fn targets() -> clap::builder::PossibleValuesParser {
    clap::builder::PossibleValuesParser::new(oxychloride::codegen::TARGETS.iter().map(|t| t.id()))
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
    let mut registry = Vec::new();
    for m in &inputs.map {
        let (from, to) = m.split_once('=').ok_or_else(|| format!("invalid --map '{m}' (expected URI=PATH)"))?;
        if Path::new(to).is_file() {
            registry.push(RegistryEntry { uri: from.to_string(), location: to.to_string() });
        } else {
            ws.map_uri(from, to);
        }
    }
    if !registry.is_empty() {
        for entry in ws.set_registry(registry) {
            if let Some(message) = entry.message {
                return Err(format!("cannot register '{}': {message}", entry.uri));
            }
            warnings.extend(entry.warnings);
        }
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

/// The project in `dir` (or with the manifest `dir`), else the one the current directory is in.
/// Its folder is relative to the current directory when it is below it.
fn find_project(dir: Option<PathBuf>) -> Result<Project, String> {
    let manifest = match dir {
        Some(dir) => dir,
        None => {
            let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
            let found = Project::find(&cwd).ok_or_else(|| {
                format!("no documents given, and no project here: no {} in this folder or above", oxychloride::MANIFEST)
            })?;
            let cwd = std::fs::canonicalize(&cwd).unwrap_or(cwd);
            match found.strip_prefix(&cwd) {
                Ok(relative) => relative.to_path_buf(),
                Err(_) => found,
            }
        }
    };
    Project::load(&manifest).map_err(|e| e.to_string())
}

/// Opens the documents of a project, with its package registry plus the `--map`s, and loads
/// the further metamodels and models of `inputs`. Returns the names of the documents.
fn load_project(
    ws: &mut Workspace,
    project: &Project,
    inputs: &Inputs,
    warnings: &mut Vec<Diagnostic>,
) -> Result<Vec<String>, String> {
    let mut project = project.clone();
    for m in &inputs.map {
        let (from, to) = m.split_once('=').ok_or_else(|| format!("invalid --map '{m}' (expected URI=PATH)"))?;
        if Path::new(to).is_file() {
            let to = std::path::absolute(to).map_err(|e| e.to_string())?;
            project.manifest.registry.insert(from.to_string(), to.to_string_lossy().into_owned());
        } else {
            ws.map_uri(from, to);
        }
    }
    let loaded = ws.open_project(&project);
    warnings.extend(loaded.problems);
    let extra = Inputs { ecore: inputs.ecore.clone(), model: inputs.model.clone(), map: Vec::new() };
    let mut documents = loaded.documents;
    documents.extend(load(ws, &extra, warnings)?);
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
        Command::Check { documents, project, inputs } => {
            let names = if documents.is_empty() {
                let project = find_project(project)?;
                let opened = load_project(&mut ws, &project, &inputs, &mut warnings)?;
                if format == Format::Text {
                    println!("{} {}", r.dim("project"), project.manifest.name);
                }
                opened
            } else {
                load(&mut ws, &inputs, &mut warnings)?;
                open_documents(&mut ws, &documents)?
            };
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
        Command::New { dir, name, description, ns_uri, ecore, with, bare, codegen, force } => {
            let name = match name {
                Some(name) => name,
                None => std::path::absolute(&dir)
                    .ok()
                    .and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()))
                    .ok_or_else(|| format!("{} has no name: give one with --name", dir.display()))?,
            };
            let include = if bare {
                Some(Vec::new())
            } else if with.is_empty() {
                None
            } else {
                Some(with.iter().map(|p| p.language()).collect())
            };
            let request = NewProjectRequest {
                name,
                description,
                ns_uri,
                metamodel: if ecore { MetamodelFormat::Ecore } else { MetamodelFormat::OclInEcore },
                include,
                codegen,
            };
            let project = Workspace::new_project(&request).map_err(|e| e.to_string())?;
            let paths: Vec<PathBuf> = project.files.iter().map(|f| dir.join(&f.path)).collect();
            if !force && let Some(existing) = paths.iter().find(|p| p.exists()) {
                return Err(format!("{} exists (use --force to replace it)", existing.display()));
            }
            for (file, path) in project.files.iter().zip(&paths) {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
                }
                std::fs::write(path, &file.text).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
            }
            if format == Format::Json {
                let files: Vec<String> = paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
                print_json(&serde_json::json!({ "package": project.package, "nsUri": project.ns_uri, "files": files }));
            } else {
                for path in &paths {
                    println!("  {}", path.display());
                }
                println!(
                    "{} created {} ({} files; package {}, nsURI {})",
                    r.ok("✓"),
                    dir.display(),
                    paths.len(),
                    project.package,
                    project.ns_uri
                );
            }
            Ok(true)
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
                (Some("henshin"), _) => ConvertTo::Henshin,
                (Some("henshin_text"), _) => ConvertTo::HenshinText,
                (_, Language::OclInEcore) => ConvertTo::Ecore,
                (_, Language::Hutn) => ConvertTo::Xmi,
                (_, Language::Henshin) => ConvertTo::Henshin,
                (_, Language::HenshinModel) => ConvertTo::HenshinText,
                _ if input_name.ends_with(".ecore") => ConvertTo::Oclinecore,
                _ => ConvertTo::Hutn,
            });
            let text = match to {
                ConvertTo::Oclinecore => {
                    ws.convert_file(&input_name, Language::OclInEcore).map_err(|e| e.to_string())?
                }
                ConvertTo::Hutn => ws.convert_file(&input_name, Language::Hutn).map_err(|e| e.to_string())?,
                ConvertTo::Ecore | ConvertTo::Xmi | ConvertTo::Henshin | ConvertTo::HenshinText => {
                    let expected = match to {
                        ConvertTo::Ecore => Language::OclInEcore,
                        ConvertTo::Xmi => Language::Hutn,
                        ConvertTo::Henshin => Language::Henshin,
                        _ => Language::HenshinModel,
                    };
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
        Command::Generate { metamodels, target, out, document, name, map } => {
            let inputs = Inputs { ecore: Vec::new(), model: Vec::new(), map };
            load(&mut ws, &inputs, &mut warnings)?;
            // The resources whose packages to generate.
            let mut resources = Vec::new();
            let mut opened = Vec::new();
            for m in &metamodels {
                if Language::from_path(&m.to_string_lossy()) == Language::OclInEcore {
                    let names = open_documents(&mut ws, std::slice::from_ref(m))?;
                    resources.extend(names.iter().cloned());
                    opened.extend(names);
                } else {
                    let r = ws.load_ecore_file(m).map_err(|e| e.to_string())?;
                    warnings.extend(r.warnings);
                    resources.push(r.uri);
                }
            }
            let documents = open_documents(&mut ws, &document)?;
            let mut diags = warnings;
            for n in opened.iter().chain(&documents) {
                diags.extend(ws.diagnostics(Some(n)));
            }
            let packages: Vec<String> =
                ws.packages().into_iter().filter(|p| resources.contains(&p.resource)).map(|p| p.name).collect();
            let request = oxychloride::CodeRequest {
                target,
                resource: None,
                packages: Some(packages),
                name,
                documents: Some(documents),
            };
            let code = ws.generate_code(&request).map_err(|e| e.to_string())?;
            for f in &code.files {
                let path = out.join(&f.path);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
                }
                std::fs::write(&path, &f.text).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
            }
            if format == Format::Json {
                let files: Vec<&str> = code.files.iter().map(|f| f.path.as_str()).collect();
                print_json(&serde_json::json!({
                    "target": code.target,
                    "packages": code.packages,
                    "files": files,
                    "problems": code.problems,
                    "diagnostics": diags,
                }));
            } else {
                let sources = sources(&ws);
                for d in &diags {
                    eprint!("{}", r.diagnostic(d, sources.get(&d.source).map(String::as_str)));
                }
                for p in &code.problems {
                    let origin = p.origin.as_deref().map(|o| format!("{o}: ")).unwrap_or_default();
                    eprintln!("{}: {origin}{}", r.warning("warning"), p.message);
                }
                println!(
                    "Generated {} files for {} into {} ({}).",
                    code.files.len(),
                    code.packages.join(", "),
                    out.display(),
                    code.target
                );
            }
            Ok(code.problems.is_empty() && !has_errors(&diags))
        }
        Command::Qvt { transformation, bind, out, target, check, config, hutn, inputs } => {
            let opened = load(&mut ws, &inputs, &mut warnings)?;
            let split = |s: &str, what: &str| -> Result<(String, String), String> {
                s.split_once('=')
                    .map(|(a, b)| (a.to_string(), b.to_string()))
                    .ok_or_else(|| format!("invalid {what} '{s}' (expected NAME=VALUE)"))
            };
            let mut models = Vec::new();
            for b in &bind {
                let (parameter, file) = split(b, "--in")?;
                let model = if Language::from_path(&file) == Language::Hutn {
                    open_documents(&mut ws, &[PathBuf::from(&file)])?.remove(0)
                } else {
                    let r = ws.load_model_file(&file).map_err(|e| e.to_string())?;
                    warnings.extend(r.warnings);
                    r.uri
                };
                models.push(oxychloride::QvtModelBinding { parameter, model });
            }
            let name = open_documents(&mut ws, std::slice::from_ref(&transformation))?.remove(0);
            if !Language::from_path(&name).is_qvt() {
                return Err(format!("{name} is not a QVT transformation (.qvto or .qvtr)"));
            }
            let mut diags = warnings;
            diags.extend(model_problems(&ws, &opened));
            diags.extend(ws.diagnostics(Some(&name)));
            let sources = sources(&ws);
            if has_errors(&diags) {
                for d in &diags {
                    eprint!("{}", r.diagnostic(d, sources.get(&d.source).map(String::as_str)));
                }
                return Ok(false);
            }
            // A single input parameter takes the model given with `-m`.
            if models.is_empty() {
                let info = ws.qvt_transformations(None).map_err(|e| e.to_string())?;
                let mut inputs: Vec<&oxychloride::QvtParameterInfo> = Vec::new();
                for t in info.iter().filter(|t| t.document == name) {
                    // QVTr: all but the direction of the run.
                    let direction = target.clone().or_else(|| (t.targets.len() == 1).then(|| t.targets[0].clone()));
                    inputs.extend(t.parameters.iter().filter(|p| match p.direction.as_deref() {
                        Some(d) => d != "out",
                        None => direction.as_deref() != Some(p.name.as_str()),
                    }));
                }
                let given: Vec<String> = ws.models().into_iter().map(|m| m.uri).collect();
                if let ([p], [m]) = (inputs.as_slice(), given.as_slice()) {
                    models.push(oxychloride::QvtModelBinding { parameter: p.name.clone(), model: m.clone() });
                }
            }
            let mut outputs = Vec::new();
            for o in &out {
                let (parameter, path) = split(o, "--out")?;
                let path = std::path::absolute(&path).map_or(path, |p| p.to_string_lossy().into_owned());
                outputs.push(oxychloride::QvtOutputPath { parameter, path });
            }
            let request = oxychloride::QvtRunRequest {
                document: name.clone(),
                models,
                outputs,
                target,
                check,
                format: hutn.then(|| "hutn".to_string()),
                configuration: config.iter().map(|c| split(c, "--config")).collect::<Result<_, _>>()?,
            };
            let result = ws.run_qvt(&request).map_err(|e| e.to_string())?;
            for o in &result.outputs {
                if out.iter().any(|x| x.split_once('=').is_some_and(|(p, _)| p == o.parameter)) {
                    std::fs::write(&o.path, &o.text).map_err(|e| format!("cannot write {}: {e}", o.path))?;
                }
            }
            let ok = result.problems.is_empty() && result.violations.is_empty() && result.aborted.is_none();
            if format == Format::Json {
                print_json(&result);
            } else {
                for d in diags.iter().chain(&result.problems) {
                    eprint!("{}", r.diagnostic(d, sources.get(&d.source).map(String::as_str)));
                }
                for line in &result.log {
                    eprintln!("{} {line}", r.dim("log:"));
                }
                if let Some(a) = &result.aborted {
                    eprintln!("{}: {a}", r.error("aborted"));
                }
                for v in &result.violations {
                    eprintln!("{} {v}", r.error("✗"));
                }
                for v in &result.invariants {
                    eprintln!("{}: {v}", r.warning("invariant"));
                }
                for o in &result.outputs {
                    if out.iter().any(|x| x.split_once('=').is_some_and(|(p, _)| p == o.parameter)) {
                        eprintln!("Wrote {} ({} objects).", o.path, o.objects);
                    } else {
                        print!("{}", o.text);
                    }
                }
                let what = if check { "checked" } else { "applied" };
                eprintln!(
                    "{} {} {what}, {} objects created{}",
                    result.applications,
                    if name.ends_with(".qvtr") { "relations" } else { "mappings" },
                    result.created,
                    if check { format!(", {} violations", result.violations.len()) } else { String::new() }
                );
            }
            Ok(ok)
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
