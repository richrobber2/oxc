//! Bounded, non-executing source validation and formatting for local tooling.
use std::{
    fmt,
    io::{self, Read},
};

use oxc_allocator::Allocator;
use oxc_diagnostics::OxcDiagnostic;
use oxc_formatter::JsFormatOptions;
use oxc_parser::{ParseOptions, Parser};
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;

/// Source bytes accepted by this helper, not a bound on parser heap or CPU use.
pub const MAX_SOURCE_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
pub enum ToolError {
    TooLarge,
    Diagnostics(Vec<OxcDiagnostic>),
    Print,
}

#[derive(Debug)]
pub struct CheckedSource {
    pub statements: usize,
    pub comments: usize,
    pub formatted: Option<String>,
    pub validation_arena_used: usize,
    pub validation_arena_capacity: usize,
}

/// Read at most the byte limit plus one byte, including for growing input.
///
/// # Errors
/// Rejects oversized input, invalid UTF-8 and underlying I/O failures.
pub fn read_bounded(reader: impl Read) -> io::Result<String> {
    let mut bytes = Vec::new();
    reader.take((MAX_SOURCE_BYTES + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "source exceeds 1 MiB"));
    }
    String::from_utf8(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "source is not UTF-8"))
}

/// Validate parser diagnostics and semantic early errors before optionally formatting.
/// No JavaScript is executed. This is syntax validation, not a vulnerability scan.
///
/// # Errors
/// Rejects oversized source, syntax/semantic diagnostics and printer failures.
pub fn process(
    source: &str,
    source_type: SourceType,
    format: bool,
) -> Result<CheckedSource, ToolError> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(ToolError::TooLarge);
    }
    // Drop the validation AST and semantic data before the formatter builds its own AST/IR.
    let (statements, comments, validation_arena_used, validation_arena_capacity) = {
        let allocator = Allocator::new();
        let parsed = Parser::new(&allocator, source, source_type)
            .with_options(ParseOptions {
                parse_regular_expression: true,
                ..ParseOptions::default()
            })
            .parse();
        if parsed.fatal_error || !parsed.diagnostics.is_empty() {
            return Err(ToolError::Diagnostics(parsed.diagnostics.into_vec()));
        }
        let semantic =
            SemanticBuilder::new_compiler().with_check_syntax_error(true).build(&parsed.program);
        if !semantic.diagnostics.is_empty() {
            return Err(ToolError::Diagnostics(semantic.diagnostics.into_vec()));
        }
        (
            parsed.program.body.len(),
            parsed.program.comments.len(),
            allocator.used_bytes(),
            allocator.capacity(),
        )
    };
    let formatted = if format {
        let allocator = Allocator::new();
        let formatted =
            oxc_formatter::format(&allocator, source, source_type, JsFormatOptions::default())
                .map_err(|error| ToolError::Diagnostics(vec![error]))?;
        Some(formatted.print().map_err(|_| ToolError::Print)?.into_code())
    } else {
        None
    };
    Ok(CheckedSource {
        statements,
        comments,
        formatted,
        validation_arena_used,
        validation_arena_capacity,
    })
}

/// Render diagnostics against borrowed source instead of copying the complete file for each error.
///
/// # Errors
/// Propagates output writer failures. Callers decide their diagnostic output cap.
pub fn render_diagnostics(
    output: &mut impl fmt::Write,
    source: &str,
    diagnostics: impl IntoIterator<Item = OxcDiagnostic>,
) -> fmt::Result {
    for diagnostic in diagnostics {
        writeln!(output, "{}", diagnostic.render_with_source_code(source))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_span::Span;

    #[test]
    fn rejects_invalid_input_and_semantic_early_errors_before_formatting() {
        for source in [
            "const value = ;",
            "let same; let same;",
            "const re = /[/;",
            "const re = /(/;",
            "const text = `unterminated",
            "<<<<<<< ours\nconst x = 1;\n=======\nconst x = 2;\n>>>>>>> theirs",
        ] {
            assert!(
                matches!(process(source, SourceType::mjs(), true), Err(ToolError::Diagnostics(errors)) if !errors.is_empty())
            );
        }
    }

    #[test]
    fn formats_unicode_comments_and_typescript_idempotently() {
        let source = "// 雪\nexport const value: { name: string } = { name: '雪' }";
        let first = process(source, SourceType::ts(), true).unwrap();
        assert_eq!(first.statements, 1);
        assert_eq!(first.comments, 1);
        let first = first.formatted.unwrap();
        assert!(first.contains("雪"));
        let checked = process(&first, SourceType::ts(), false).unwrap();
        assert_eq!((checked.statements, checked.comments), (1, 1));
        assert_eq!(process(&first, SourceType::ts(), true).unwrap().formatted.unwrap(), first);
    }

    #[test]
    fn input_limits_apply_to_readers_and_direct_calls() {
        let oversized = " ".repeat(MAX_SOURCE_BYTES + 1);
        assert!(read_bounded(oversized.as_bytes()).is_err());
        assert!(matches!(process(&oversized, SourceType::mjs(), false), Err(ToolError::TooLarge)));
        assert_eq!(
            read_bounded(&oversized.as_bytes()[..MAX_SOURCE_BYTES]).unwrap().len(),
            MAX_SOURCE_BYTES
        );
        assert!(read_bounded(&[0xff][..]).is_err());
    }

    #[test]
    fn borrowed_diagnostics_match_owned_source_rendering() {
        let source = "const 雪 = 1;\r\n";
        let errors = [OxcDiagnostic::error("synthetic failure").with_label(Span::new(6, 9))];
        let expected =
            format!("{}\n", errors[0].clone().render_with_source_code(source.to_owned()));
        let mut actual = String::new();
        render_diagnostics(&mut actual, source, errors).unwrap();
        assert_eq!(actual, expected);
    }
}
