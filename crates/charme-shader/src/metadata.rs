use std::{fmt, ops::Range};

use nom::{
    Err as NomErr, IResult, Parser,
    branch::alt,
    bytes::complete::tag,
    character::complete::{char, multispace0, one_of, space0},
    combinator::{all_consuming, cut, map, opt, recognize},
    error::{ContextError, ErrorKind, ParseError as NomParseError, context},
    multi::{many0, separated_list1},
    number::complete::recognize_float,
    sequence::{delimited, preceded, terminated},
};

/// A parsed `%{ ... }` metadata block.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MetadataBlock {
    pub entries: Vec<MetadataEntry>,
}

impl MetadataBlock {
    /// Returns the last entry matching a dotted path.
    ///
    /// Returning the last entry makes source-order overrides possible while
    /// preserving all entries for consumers that want to diagnose duplicates.
    pub fn get(&self, path: &str) -> Option<&MetadataValue> {
        self.entries
            .iter()
            .rev()
            .find(|entry| entry.path.matches(path))
            .map(|entry| &entry.value)
    }
}

/// One metadata assignment or flag.
#[derive(Clone, Debug, PartialEq)]
pub struct MetadataEntry {
    pub path: MetadataPath,
    pub value: MetadataValue,
    /// Byte range of this entry within the parsed metadata block.
    pub span: Range<usize>,
}

/// A dotted metadata path such as `ui.label` or `reflect.parameters`.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct MetadataPath {
    pub segments: Vec<String>,
}

impl MetadataPath {
    pub fn as_slice(&self) -> &[String] {
        &self.segments
    }

    pub fn matches(&self, dotted_path: &str) -> bool {
        self.segments
            .iter()
            .map(String::as_str)
            .eq(dotted_path.split('.'))
    }
}

impl fmt::Display for MetadataPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut segments = self.segments.iter();
        if let Some(first) = segments.next() {
            formatter.write_str(first)?;
        }
        for segment in segments {
            write!(formatter, ".{segment}")?;
        }
        Ok(())
    }
}

/// A value supported by the metadata language.
#[derive(Clone, Debug, PartialEq)]
pub enum MetadataValue {
    /// The value of a path written without `= value`.
    Bool(bool),
    Integer(i64),
    Float(f64),
    String(String),
    Identifier(String),
}

/// An error produced while parsing a `%{ ... }` metadata block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetadataParseError {
    /// Byte offset in the input at which parsing failed.
    pub offset: usize,
    /// The grammar construct being parsed when the error occurred.
    pub context: &'static str,
}

impl fmt::Display for MetadataParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid metadata {} at byte offset {}",
            self.context, self.offset
        )
    }
}

impl std::error::Error for MetadataParseError {}

/// Parses one complete `%{ ... }` metadata block.
///
/// A bare path is a boolean flag with the value `true`. Statements are
/// separated by a newline or semicolon. Both compact and multiline forms are
/// accepted:
///
/// ```text
/// %{ ui.color; ui.space = "linear"; }
/// ```
///
/// ```text
/// %{
///   reflect.parameters
///   ui.label = "Material"
/// }
/// ```
///
/// The input should contain the metadata block itself, without WGSL `///`
/// prefixes. Whitespace may surround the block, but other text is rejected.
pub fn parse_metadata_block(input: &str) -> Result<MetadataBlock, MetadataParseError> {
    let mut parser = all_consuming(delimited(
        preceded(multispace0, tag("%{")),
        preceded(
            multispace0,
            map(
                (
                    many0(terminated(metadata_entry(input), statement_separator)),
                    opt(metadata_entry(input)),
                ),
                |(mut entries, last)| {
                    entries.extend(last);
                    MetadataBlock { entries }
                },
            ),
        ),
        terminated(
            preceded(multispace0, context("closing `}`", char('}'))),
            multispace0,
        ),
    ));

    match parser.parse(input) {
        Ok((_, block)) => Ok(block),
        Err(NomErr::Error(error) | NomErr::Failure(error)) => Err(MetadataParseError {
            offset: input.len() - error.input.len(),
            context: error.context,
        }),
        Err(NomErr::Incomplete(_)) => unreachable!("complete parsers never return Incomplete"),
    }
}

fn metadata_entry<'source>(
    source: &'source str,
) -> impl Parser<&'source str, Output = MetadataEntry, Error = InternalParseError<'source>> {
    move |input: &'source str| {
        let start = source.len() - input.len();
        let (input, _) = space0.parse(input)?;
        let (input, path) = context("path", metadata_path).parse(input)?;
        let (input, value) = opt(preceded(
            (space0, char('='), space0),
            cut(context("value", metadata_value)),
        ))
        .parse(input)?;
        let (input, _) = space0.parse(input)?;
        let end = source.len() - input.len();

        Ok((
            input,
            MetadataEntry {
                path,
                value: value.unwrap_or(MetadataValue::Bool(true)),
                span: start..end,
            },
        ))
    }
}

fn metadata_path(input: &str) -> ParseResult<'_, MetadataPath> {
    map(separated_list1(char('.'), identifier), |segments| {
        MetadataPath {
            segments: segments.into_iter().map(str::to_owned).collect(),
        }
    })
    .parse(input)
}

fn metadata_value(input: &str) -> ParseResult<'_, MetadataValue> {
    alt((
        map(quoted_string, MetadataValue::String),
        number,
        map(identifier, |identifier| match identifier {
            "true" => MetadataValue::Bool(true),
            "false" => MetadataValue::Bool(false),
            value => MetadataValue::Identifier(value.to_owned()),
        }),
    ))
    .parse(input)
}

fn number(input: &str) -> ParseResult<'_, MetadataValue> {
    let (remaining, literal) = recognize_float.parse(input)?;
    let value = if literal.contains(['.', 'e', 'E']) {
        literal
            .parse::<f64>()
            .map(MetadataValue::Float)
            .map_err(|_| ())
    } else {
        literal
            .parse::<i64>()
            .map(MetadataValue::Integer)
            .map_err(|_| ())
    }
    .map_err(|_| NomErr::Failure(InternalParseError::new(input, "number")))?;

    Ok((remaining, value))
}

fn identifier(input: &str) -> ParseResult<'_, &str> {
    recognize((
        one_of("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ_"),
        many0(one_of(
            "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ_0123456789-",
        )),
    ))
    .parse(input)
}

fn quoted_string(input: &str) -> ParseResult<'_, String> {
    if !input.starts_with('"') {
        return Err(NomErr::Error(InternalParseError::new(
            input,
            "string literal",
        )));
    }

    let mut output = String::new();
    let mut chars = input[1..].char_indices();

    while let Some((relative_offset, character)) = chars.next() {
        let absolute_offset = 1 + relative_offset;
        match character {
            '"' => return Ok((&input[absolute_offset + 1..], output)),
            '\\' => {
                let Some((escape_offset, escaped)) = chars.next() else {
                    return Err(NomErr::Failure(InternalParseError::new(
                        &input[absolute_offset..],
                        "string escape",
                    )));
                };
                match escaped {
                    '"' => output.push('"'),
                    '\\' => output.push('\\'),
                    'n' => output.push('\n'),
                    'r' => output.push('\r'),
                    't' => output.push('\t'),
                    '0' => output.push('\0'),
                    _ => {
                        return Err(NomErr::Failure(InternalParseError::new(
                            &input[1 + escape_offset..],
                            "string escape",
                        )));
                    }
                }
            }
            '\n' | '\r' => {
                return Err(NomErr::Failure(InternalParseError::new(
                    &input[absolute_offset..],
                    "closing quote",
                )));
            }
            value => output.push(value),
        }
    }

    Err(NomErr::Failure(InternalParseError::new(
        &input[input.len()..],
        "closing quote",
    )))
}

fn statement_separator(input: &str) -> ParseResult<'_, &str> {
    let mut found_separator = false;
    let mut end = 0;

    for (offset, character) in input.char_indices() {
        if character == ';' || character == '\n' || character == '\r' {
            found_separator = true;
            end = offset + character.len_utf8();
        } else if character == ' ' || character == '\t' {
            end = offset + character.len_utf8();
        } else {
            break;
        }
    }

    if found_separator {
        Ok((&input[end..], &input[..end]))
    } else {
        Err(NomErr::Error(InternalParseError::new(
            input,
            "statement separator",
        )))
    }
}

type ParseResult<'source, Output> = IResult<&'source str, Output, InternalParseError<'source>>;

#[derive(Clone, Debug, Eq, PartialEq)]
struct InternalParseError<'source> {
    input: &'source str,
    context: &'static str,
}

impl<'source> InternalParseError<'source> {
    fn new(input: &'source str, context: &'static str) -> Self {
        Self { input, context }
    }

    fn furthest(self, other: Self) -> Self {
        if other.input.len() < self.input.len() {
            other
        } else {
            self
        }
    }
}

impl<'source> NomParseError<&'source str> for InternalParseError<'source> {
    fn from_error_kind(input: &'source str, _kind: ErrorKind) -> Self {
        Self::new(input, "syntax")
    }

    fn append(input: &'source str, _kind: ErrorKind, other: Self) -> Self {
        other.furthest(Self::new(input, "syntax"))
    }

    fn or(self, other: Self) -> Self {
        self.furthest(other)
    }
}

impl<'source> ContextError<&'source str> for InternalParseError<'source> {
    fn add_context(input: &'source str, context: &'static str, other: Self) -> Self {
        if input.len() <= other.input.len() {
            Self::new(input, context)
        } else {
            other
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_empty_block() {
        assert_eq!(
            parse_metadata_block("%{ }").unwrap(),
            MetadataBlock::default()
        );
    }

    #[test]
    fn parses_compact_flags_and_assignments() {
        let block = parse_metadata_block(
            r#"%{ reflect.parameters; ui.label = "Material"; ui.color; ui.space = linear; }"#,
        )
        .unwrap();

        assert_eq!(block.entries.len(), 4);
        assert_eq!(
            block.get("reflect.parameters"),
            Some(&MetadataValue::Bool(true))
        );
        assert_eq!(
            block.get("ui.label"),
            Some(&MetadataValue::String("Material".to_owned()))
        );
        assert_eq!(block.get("ui.color"), Some(&MetadataValue::Bool(true)));
        assert_eq!(
            block.get("ui.space"),
            Some(&MetadataValue::Identifier("linear".to_owned()))
        );
    }

    #[test]
    fn parses_multiline_typed_values() {
        let block = parse_metadata_block(
            r#"
                %{
                    ui.min = -2
                    ui.max = 1.5
                    ui.step = 1e-3
                    ui.hidden = false
                    ui.label = "Line\nBreak"
                }
            "#,
        )
        .unwrap();

        assert_eq!(block.get("ui.min"), Some(&MetadataValue::Integer(-2)));
        assert_eq!(block.get("ui.max"), Some(&MetadataValue::Float(1.5)));
        assert_eq!(block.get("ui.step"), Some(&MetadataValue::Float(1e-3)));
        assert_eq!(block.get("ui.hidden"), Some(&MetadataValue::Bool(false)));
        assert_eq!(
            block.get("ui.label"),
            Some(&MetadataValue::String("Line\nBreak".to_owned()))
        );
    }

    #[test]
    fn parses_unicode_and_string_escapes() {
        let block =
            parse_metadata_block(r#"%{ ui.label = "曝光 \"EV\""; ui.path = "a\\b"; }"#).unwrap();

        assert_eq!(
            block.get("ui.label"),
            Some(&MetadataValue::String("曝光 \"EV\"".to_owned()))
        );
        assert_eq!(
            block.get("ui.path"),
            Some(&MetadataValue::String("a\\b".to_owned()))
        );
    }

    #[test]
    fn later_entries_override_get_without_being_discarded() {
        let block = parse_metadata_block("%{ ui.min = 0; ui.min = 1; }").unwrap();

        assert_eq!(block.entries.len(), 2);
        assert_eq!(block.get("ui.min"), Some(&MetadataValue::Integer(1)));
    }

    #[test]
    fn records_entry_spans() {
        let source = "%{ ui.color;\nui.min = 0\n}";
        let block = parse_metadata_block(source).unwrap();

        assert_eq!(&source[block.entries[0].span.clone()], "ui.color");
        assert_eq!(&source[block.entries[1].span.clone()], "ui.min = 0");
    }

    #[test]
    fn rejects_missing_statement_separator() {
        let error = parse_metadata_block("%{ ui.min = 0 ui.max = 1 }").unwrap_err();
        assert!(error.offset >= "%{ ui.min = 0".len());
    }

    #[test]
    fn rejects_invalid_escape() {
        let error = parse_metadata_block(r#"%{ ui.label = "bad\q"; }"#).unwrap_err();
        assert_eq!(error.context, "string escape");
    }

    #[test]
    fn rejects_unterminated_block() {
        let error = parse_metadata_block("%{ ui.color").unwrap_err();
        assert_eq!(error.context, "closing `}`");
    }

    #[test]
    fn rejects_text_outside_block() {
        assert!(parse_metadata_block("description %{ ui.color }").is_err());
        assert!(parse_metadata_block("%{ ui.color } trailing").is_err());
    }
}
