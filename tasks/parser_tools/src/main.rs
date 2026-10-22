use std::{
    env,
    fs::OpenOptions,
    io::{self, Write},
    path::Path,
    process::ExitCode,
};

use oxc_parser_tools::{ToolError, process, read_bounded, render_diagnostics};
use oxc_span::SourceType;

fn run() -> Result<(), String> {
    let args: Vec<_> = env::args().skip(1).collect();
    if args.len() != 2 || !matches!(args[0].as_str(), "check" | "format") {
        return Err("usage: oxc-parser-tools <check|format> <source.js|source.ts|source.tsx>; writes stdout only".into());
    }
    let path = Path::new(&args[1]);
    let source_type = SourceType::from_path(path).map_err(|_| "unsupported source extension")?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Do not block on a FIFO before checking the opened handle's file type.
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|_| "cannot open source")?;
    if !file.metadata().map_err(|_| "cannot stat source")?.is_file() {
        return Err("source must be a regular file".into());
    }
    let source = read_bounded(file).map_err(|error| error.to_string())?;
    match process(&source, source_type, args[0] == "format") {
        Ok(result) => {
            let output = result.formatted.unwrap_or_else(|| {
                let summary = serde_json::json!({
                    "syntaxValid": true,
                    "statements": result.statements,
                    "comments": result.comments,
                    "validationArenaUsed": result.validation_arena_used,
                    "validationArenaCapacity": result.validation_arena_capacity,
                });
                format!("{summary}\n")
            });
            let mut stdout = io::stdout().lock();
            stdout.write_all(output.as_bytes()).map_err(|_| "cannot write output")?;
            stdout.flush().map_err(|_| "cannot flush output")?;
            Ok(())
        }
        Err(ToolError::Diagnostics(errors)) => {
            let count = errors.len();
            let mut output = String::new();
            render_diagnostics(&mut output, &source, errors.into_iter().take(10))
                .map_err(|_| "cannot render diagnostic")?;
            Err(format!("{output}{count} syntax diagnostics (at most 10 displayed)"))
        }
        Err(ToolError::TooLarge) => Err("source exceeds 1 MiB".into()),
        Err(ToolError::Print) => Err("formatting failed".into()),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            let _ = writeln!(io::stderr().lock(), "{message}");
            ExitCode::FAILURE
        }
    }
}
