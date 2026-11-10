//! DiagramCraft: open-source diagramming, rebuilt in pure Rust.
//!
//! ```text
//! diagramcraft                            open the editor
//! diagramcraft FILE.lucid                 open the editor on a document
//! diagramcraft export IN.lucid OUT.png [SCALE]
//!                                         render headlessly to a PNG
//! diagramcraft demo OUT.lucid             write the bundled sample document
//! ```

mod app;
mod clipboard;
mod editor;
mod export;
mod history;
mod menu;
#[cfg(target_os = "macos")]
mod native_menu;
mod samples;
mod settings;
mod snap;
mod textedit;
mod ui;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
DiagramCraft — open-source diagramming in pure Rust

USAGE:
    diagramcraft [FILE.lucid | FILE.vsdx]
    diagramcraft export <IN.lucid> <OUT.png> [SCALE]
    diagramcraft import <IN.vsdx> <OUT.lucid>
    diagramcraft demo <OUT.lucid>
    diagramcraft preview <IN.lucid> <OUT.png> [--dark] [--editing]
                                              [--menu NAME] [--library NAME]
                                              [--settings] [--bold] [--line]
    diagramcraft --help
";

fn main() -> ExitCode {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn,dc_app=info"),
    )
    .init();

    let args: Vec<String> = std::env::args().skip(1).collect();

    match args.first().map(String::as_str) {
        Some("--help" | "-h" | "help") => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some("--version" | "-V") => {
            println!("diagramcraft {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("export") => run_export(&args[1..]),
        Some("import") => run_import(&args[1..]),
        Some("demo") => run_demo(&args[1..]),
        Some("preview") => run_preview(&args[1..]),
        _ => run_editor(args.first().map(PathBuf::from)),
    }
}

fn run_editor(path: Option<PathBuf>) -> ExitCode {
    let mut application = app::App::new();
    if let Some(path) = path {
        application.open_initial(&path);
    }
    match application.run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("diagramcraft: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_export(args: &[String]) -> ExitCode {
    let (Some(input), Some(output)) = (args.first(), args.get(1)) else {
        eprintln!("diagramcraft export: expected <IN.lucid> <OUT.png> [SCALE]\n\n{USAGE}");
        return ExitCode::FAILURE;
    };

    let scale = match args.get(2) {
        Some(raw) => match raw.parse::<f64>() {
            Ok(s) if s > 0.0 && s.is_finite() => s,
            _ => {
                eprintln!("diagramcraft export: SCALE must be a positive number, got {raw:?}");
                return ExitCode::FAILURE;
            }
        },
        None => 2.0,
    };

    let package = match dc_lucid::LucidPackage::read_path(Path::new(input)) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("diagramcraft: could not read {input}: {e}");
            return ExitCode::FAILURE;
        }
    };

    match export::render_png(&package, 0, scale, Path::new(output)) {
        Ok((w, h)) => {
            println!("wrote {output} ({w}x{h})");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("diagramcraft: export failed: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Converts a Visio drawing to a `.lucid` document, headlessly.
fn run_import(args: &[String]) -> ExitCode {
    let (Some(input), Some(output)) = (args.first(), args.get(1)) else {
        eprintln!("diagramcraft import: expected <IN.vsdx> <OUT.lucid>\n\n{USAGE}");
        return ExitCode::FAILURE;
    };

    let imported = match dc_vsdx::read_path(Path::new(input)) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("diagramcraft: could not read {input}: {e}");
            return ExitCode::FAILURE;
        }
    };
    for warning in &imported.warnings {
        eprintln!("diagramcraft: {warning}");
    }
    let summary = imported.summary();

    let package = dc_lucid::LucidPackage::new(imported.document);
    match package.write_path(Path::new(output)) {
        Ok(()) => {
            println!("wrote {output}: {summary}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("diagramcraft: could not write {output}: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Renders the editor interface itself, for documentation and visual review.
fn run_preview(args: &[String]) -> ExitCode {
    let (Some(input), Some(output)) = (args.first(), args.get(1)) else {
        eprintln!("diagramcraft preview: expected <IN.lucid> <OUT.png> [--dark]\n\n{USAGE}");
        return ExitCode::FAILURE;
    };
    let dark = args.iter().any(|a| a == "--dark");
    let editing = args.iter().any(|a| a == "--editing");
    let settings_open = args.iter().any(|a| a == "--settings");

    // `--menu NAME` drops a menu open, and `--library NAME` switches the
    // palette, so both can be reviewed in a screenshot without a display.
    let flag_value = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .map(String::as_str)
    };
    let open_menu = flag_value("--menu").and_then(|name| {
        const TITLES: [&str; 8] = [
            "file", "edit", "select", "insert", "format", "arrange", "view", "help",
        ];
        TITLES.iter().position(|t| t.eq_ignore_ascii_case(name))
    });
    let library = flag_value("--library")
        .and_then(|name| {
            dc_model::ShapeLibrary::ALL
                .iter()
                .copied()
                .find(|l| l.short_name().eq_ignore_ascii_case(name))
        })
        .unwrap_or(dc_model::ShapeLibrary::Standard);

    let mut editor = editor::Editor::new();
    if let Err(e) = editor.open_path(Path::new(input)) {
        eprintln!("diagramcraft: could not read {input}: {e}");
        return ExitCode::FAILURE;
    }

    // `--line` selects the first connector, so the panel's connector rows
    // can be reviewed the same way its shape rows can.
    if args.iter().any(|a| a == "--line") {
        if let Some(id) = editor.page().lines().first().map(|l| l.id.clone()) {
            editor.selection = vec![id];
        }
    }

    // Opens the first label so a screenshot can show the editing state.
    if editing {
        let mut fonts = parley::FontContext::new();
        let mut layouts = parley::LayoutContext::new();
        let mut cx = textedit::TextCtx::new(&mut fonts, &mut layouts);
        if let Some(id) = editor.page().shapes().first().map(|s| s.id.clone()) {
            editor.selection = vec![id];
            editor.begin_text_edit_on_selection(&mut cx);
            // Select the first word, which is the state worth showing: a
            // caret in a label with part of it highlighted.
            editor.text_command(
                textedit::TextCommand::TextStart { select: false },
                &mut cx,
            );
            editor.text_command(
                textedit::TextCommand::Right { select: true, word: true },
                &mut cx,
            );
            // `--bold` styles just that word, which is what proves the
            // formatting reaches the selection rather than the whole label.
            if args.iter().any(|a| a == "--bold") {
                editor.toggle_text_style(editor::TextToggle::Bold);
            }
        }
    }

    match export::render_editor_png(
        &mut editor,
        kurbo::Vec2::new(1440.0, 900.0),
        2.0,
        dark,
        library,
        open_menu,
        settings_open,
        Path::new(output),
    ) {
        Ok((w, h)) => {
            println!("wrote {output} ({w}x{h})");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("diagramcraft: preview failed: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_demo(args: &[String]) -> ExitCode {
    let Some(output) = args.first() else {
        eprintln!("diagramcraft demo: expected <OUT.lucid>\n\n{USAGE}");
        return ExitCode::FAILURE;
    };

    let package = dc_lucid::LucidPackage::new(samples::approval_flow());
    match package.write_path(Path::new(output)) {
        Ok(()) => {
            println!("wrote {output}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("diagramcraft: could not write {output}: {e}");
            ExitCode::FAILURE
        }
    }
}
