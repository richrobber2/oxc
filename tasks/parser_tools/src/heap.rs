//! Dedicated DHAT process; normal tools keep their existing allocator.
use std::{
    fmt::{self, Write as _},
    fs,
    process::ExitCode,
};

use oxc_diagnostics::OxcDiagnostic;
use oxc_parser_tools::{ToolError, process, render_diagnostics};
use oxc_span::{SourceType, Span};

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

struct HashSink(u64);
impl HashSink {
    const fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}
impl fmt::Write for HashSink {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        for byte in text.bytes() {
            self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3);
        }
        Ok(())
    }
}

struct Fixture {
    case: String,
    source: String,
    source_type: SourceType,
    diagnostics: Vec<OxcDiagnostic>,
}
impl Fixture {
    fn new(case: String) -> Result<Self, String> {
        let source_type = if case.ends_with("-ts") { SourceType::ts() } else { SourceType::mjs() };
        let source = match case.as_str() {
            "check-js" | "check-ts" | "format-js" | "format-ts" => {
                let parameter = if source_type.is_typescript() { "value: string" } else { "value" };
                let mut source = String::new();
                for i in 0..256 {
                    writeln!(source, "// synthetic 雪\nexport function item{i}({parameter}){{return {{text:'雪',value,re:/[a-z]+/u.test(value)}};}}")
                        .map_err(|_| "fixture generation failed")?;
                }
                source
            }
            "reject-malformed" => "export const value = ;".into(),
            "diagnostics-owned" | "diagnostics-borrowed" => "const item = 1;\n".to_owned() + &"// synthetic padding\n".repeat(40_000),
            _ => return Err("choose check-js, check-ts, format-js, format-ts, reject-malformed, diagnostics-owned or diagnostics-borrowed".into()),
        };
        let diagnostics = if case.starts_with("diagnostics-") {
            std::iter::repeat_with(|| {
                OxcDiagnostic::error("synthetic finding").with_label(Span::new(0, 5))
            })
            .take(64)
            .collect()
        } else {
            Vec::new()
        };
        Ok(Self { case, source, source_type, diagnostics })
    }

    fn run(&self) -> Result<u64, String> {
        let mut sink = HashSink::new();
        match self.case.as_str() {
            "diagnostics-owned" => {
                for error in &self.diagnostics {
                    writeln!(
                        sink,
                        "{}",
                        error.clone().render_with_source_code(self.source.clone())
                    )
                    .map_err(|_| "hash writer failed")?;
                }
            }
            "diagnostics-borrowed" => {
                render_diagnostics(&mut sink, &self.source, self.diagnostics.iter().cloned())
                    .map_err(|_| "hash writer failed")?;
            }
            "reject-malformed" => match process(&self.source, self.source_type, true) {
                Err(ToolError::Diagnostics(errors)) if !errors.is_empty() => {
                    for error in errors {
                        write!(sink, "{error}").map_err(|_| "hash writer failed")?;
                    }
                }
                _ => return Err("malformed fixture was not rejected".into()),
            },
            _ => {
                let result =
                    process(&self.source, self.source_type, self.case.starts_with("format-"))
                        .map_err(|_| "valid fixture was rejected")?;
                if result.statements != 256 || result.comments != 256 {
                    return Err("fixture lost statements or comments".into());
                }
                write!(sink, "{}:{}", result.statements, result.comments)
                    .map_err(|_| "hash writer failed")?;
                if let Some(code) = result.formatted {
                    sink.write_str(&code).map_err(|_| "hash writer failed")?;
                }
            }
        }
        Ok(std::hint::black_box(sink.0))
    }

    fn validate(&self) -> Result<(), String> {
        if self.case.starts_with("format-") {
            let first = process(&self.source, self.source_type, true)
                .map_err(|_| "fixture formatting failed")?
                .formatted
                .ok_or("missing formatted output")?;
            let checked = process(&first, self.source_type, true)
                .map_err(|_| "formatted fixture was invalid")?;
            if checked.statements != 256 || checked.comments != 256 {
                return Err("formatted fixture lost statements or comments".into());
            }
            let second = checked.formatted.ok_or("missing second output")?;
            if first != second {
                return Err("formatting is not idempotent".into());
            }
        }
        Ok(())
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: oxc-parser-tools-heap <case> <iterations:1..100>".into());
    }
    let iterations: usize = args[1].parse()?;
    if !(1..=100).contains(&iterations) {
        return Err("iterations must be 1..100".into());
    }
    let fixture = Fixture::new(args[0].clone())?;
    fixture.validate()?;
    let arena = if fixture.case.starts_with("check-") || fixture.case.starts_with("format-") {
        let checked = process(&fixture.source, fixture.source_type, false)
            .map_err(|_| "arena preflight failed")?;
        Some((checked.validation_arena_used, checked.validation_arena_capacity))
    } else {
        None
    };
    // Establish the full-output checksum and warm lazy state outside profiling.
    let expected = fixture.run()?;
    let directory = std::path::Path::new("target/parser-tool-profiles");
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(directory)?;
    let path = directory.join(format!("{}-{}.json", args[0], std::process::id()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    drop(options.open(&path)?);
    let profiler = dhat::Profiler::builder().file_name(&path).trim_backtraces(Some(32)).build();
    let mut valid = true;
    for _ in 0..iterations {
        valid &= fixture.run()? == expected;
    }
    let stats = dhat::HeapStats::get();
    drop(profiler);
    if !valid || fs::metadata(&path)?.len() == 0 {
        return Err("checksum mismatch or missing DHAT report".into());
    }
    let summary = serde_json::json!({"case":args[0],"iterations":iterations,"checksum":expected,"totalBytes":stats.total_bytes,"totalBlocks":stats.total_blocks,"peakBytes":stats.max_bytes,"endBytes":stats.curr_bytes,"validationArena":arena,"report":path});
    let mut output = std::io::stdout().lock();
    std::io::Write::write_all(&mut output, format!("{summary}\n").as_bytes())?;
    Ok(())
}
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ =
                std::io::Write::write_fmt(&mut std::io::stderr().lock(), format_args!("{error}\n"));
            ExitCode::FAILURE
        }
    }
}
