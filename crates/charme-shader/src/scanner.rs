use std::ops::Range;

use unicode_ident::{is_xid_continue, is_xid_start};

use crate::{MetadataBlock, MetadataParseError, parse_metadata_block};

/// Metadata and documentation discovered in one preprocessed WGSL module.
#[derive(Clone, Debug, PartialEq)]
pub struct ModuleMetadata {
    /// Caller-provided module identity, such as a file path or naga-oil import path.
    pub module: String,
    pub declarations: Vec<DeclarationMetadata>,
    pub diagnostics: Vec<ScanDiagnostic>,
}

impl ModuleMetadata {
    pub fn get(&self, declaration: &SourceDeclaration) -> Option<&DeclarationMetadata> {
        self.declarations
            .iter()
            .find(|metadata| &metadata.declaration == declaration)
    }
}

/// Documentation attached to a source declaration.
#[derive(Clone, Debug, PartialEq)]
pub struct DeclarationMetadata {
    pub declaration: SourceDeclaration,
    /// Documentation outside `%{ ... }` blocks.
    pub description: String,
    pub metadata: MetadataBlock,
    /// Byte range covering the source doc comments.
    pub documentation_span: Range<usize>,
    /// Byte range of the declaration's identifier.
    pub declaration_span: Range<usize>,
}

/// A declaration target, qualified by the identity on its parent
/// [`ModuleMetadata`], before it is mapped to Naga IR.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum SourceDeclaration {
    Struct { name: String },
    StructMember { structure: String, member: String },
    GlobalVariable { name: String },
    Override { name: String },
    Constant { name: String },
}

/// A recoverable problem found while scanning source metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanDiagnostic {
    pub span: Range<usize>,
    pub kind: ScanDiagnosticKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScanDiagnosticKind {
    InvalidMetadata { context: &'static str },
    UnterminatedMetadataBlock,
    MetadataOnUnsupportedDeclaration,
    UnterminatedBlockComment,
}

/// Scans preprocessed WGSL source and attaches doc-comment metadata to source
/// declarations.
///
/// This scanner deliberately does not parse WGSL types or expressions. It only
/// recognizes module-level structs, globals, overrides and constants, plus
/// struct members. Naga remains the authority for syntax, types and layout.
/// The source should first be processed with the same naga-oil shader defs as
/// the module that will be reflected.
pub fn scan_module_metadata(module: impl Into<String>, source: &str) -> ModuleMetadata {
    let (tokens, mut diagnostics) = lex(source);
    let mut declarations = Vec::new();
    let mut index = 0;

    while index < tokens.len() {
        let (documentation, after_documentation) = take_documentation(&tokens, index);
        let declaration_index = skip_attributes(&tokens, after_documentation);

        if is_identifier(&tokens, declaration_index, source, "struct") {
            index = scan_struct(
                &tokens,
                declaration_index,
                documentation,
                source,
                &mut declarations,
                &mut diagnostics,
            );
        } else if is_identifier(&tokens, declaration_index, source, "var") {
            index = scan_global(
                &tokens,
                declaration_index,
                documentation,
                source,
                &mut declarations,
                &mut diagnostics,
            );
        } else if is_identifier(&tokens, declaration_index, source, "override") {
            index = scan_named_module_item(
                &tokens,
                declaration_index,
                documentation,
                source,
                &mut declarations,
                &mut diagnostics,
                |name| SourceDeclaration::Override { name },
            );
        } else if is_identifier(&tokens, declaration_index, source, "const") {
            index = scan_named_module_item(
                &tokens,
                declaration_index,
                documentation,
                source,
                &mut declarations,
                &mut diagnostics,
                |name| SourceDeclaration::Constant { name },
            );
        } else if is_identifier(&tokens, declaration_index, source, "fn") {
            report_unsupported_metadata(documentation, source, &mut diagnostics);
            index = skip_function(&tokens, declaration_index);
        } else {
            report_unsupported_metadata(documentation, source, &mut diagnostics);
            index = if declaration_index > index {
                declaration_index
            } else {
                index + 1
            };
        }
    }

    ModuleMetadata {
        module: module.into(),
        declarations,
        diagnostics,
    }
}

fn scan_struct(
    tokens: &[Token],
    keyword_index: usize,
    documentation: Option<RawDocumentation>,
    source: &str,
    declarations: &mut Vec<DeclarationMetadata>,
    diagnostics: &mut Vec<ScanDiagnostic>,
) -> usize {
    let Some(name_token) = tokens
        .get(keyword_index + 1)
        .filter(|token| token.is_identifier())
    else {
        report_unsupported_metadata(documentation, source, diagnostics);
        return keyword_index + 1;
    };
    let structure = name_token.text(source).to_owned();
    let Some(open_index) = tokens
        .get(keyword_index + 2)
        .filter(|token| token.is_symbol('{'))
        .map(|_| keyword_index + 2)
    else {
        report_unsupported_metadata(documentation, source, diagnostics);
        return keyword_index + 1;
    };

    attach_documentation(
        documentation,
        SourceDeclaration::Struct {
            name: structure.clone(),
        },
        name_token.span.clone(),
        source,
        declarations,
        diagnostics,
    );

    let mut index = open_index + 1;
    while index < tokens.len() {
        if tokens[index].is_symbol('}') {
            return index + 1;
        }

        let (member_documentation, after_documentation) = take_documentation(tokens, index);
        let member_index = skip_attributes(tokens, after_documentation);

        let Some(member_token) = tokens
            .get(member_index)
            .filter(|token| token.is_identifier())
        else {
            report_unsupported_metadata(member_documentation, source, diagnostics);
            index = if member_index > index {
                member_index
            } else {
                index + 1
            };
            continue;
        };

        if !tokens
            .get(member_index + 1)
            .is_some_and(|token| token.is_symbol(':'))
        {
            report_unsupported_metadata(member_documentation, source, diagnostics);
            index = member_index + 1;
            continue;
        }

        attach_documentation(
            member_documentation,
            SourceDeclaration::StructMember {
                structure: structure.clone(),
                member: member_token.text(source).to_owned(),
            },
            member_token.span.clone(),
            source,
            declarations,
            diagnostics,
        );

        index = skip_to_delimiter(tokens, member_index + 2, ',', '}');
        if tokens.get(index).is_some_and(|token| token.is_symbol(',')) {
            index += 1;
        }
    }

    tokens.len()
}

fn scan_global(
    tokens: &[Token],
    keyword_index: usize,
    documentation: Option<RawDocumentation>,
    source: &str,
    declarations: &mut Vec<DeclarationMetadata>,
    diagnostics: &mut Vec<ScanDiagnostic>,
) -> usize {
    let mut name_index = keyword_index + 1;
    if tokens
        .get(name_index)
        .is_some_and(|token| token.is_symbol('<'))
    {
        name_index = skip_balanced(tokens, name_index, '<', '>');
    }

    let Some(name_token) = tokens.get(name_index).filter(|token| token.is_identifier()) else {
        report_unsupported_metadata(documentation, source, diagnostics);
        return keyword_index + 1;
    };

    attach_documentation(
        documentation,
        SourceDeclaration::GlobalVariable {
            name: name_token.text(source).to_owned(),
        },
        name_token.span.clone(),
        source,
        declarations,
        diagnostics,
    );

    skip_to_semicolon(tokens, name_index + 1)
}

fn scan_named_module_item(
    tokens: &[Token],
    keyword_index: usize,
    documentation: Option<RawDocumentation>,
    source: &str,
    declarations: &mut Vec<DeclarationMetadata>,
    diagnostics: &mut Vec<ScanDiagnostic>,
    make_declaration: impl FnOnce(String) -> SourceDeclaration,
) -> usize {
    let Some(name_token) = tokens
        .get(keyword_index + 1)
        .filter(|token| token.is_identifier())
    else {
        report_unsupported_metadata(documentation, source, diagnostics);
        return keyword_index + 1;
    };

    attach_documentation(
        documentation,
        make_declaration(name_token.text(source).to_owned()),
        name_token.span.clone(),
        source,
        declarations,
        diagnostics,
    );

    skip_to_semicolon(tokens, keyword_index + 2)
}

fn attach_documentation(
    documentation: Option<RawDocumentation>,
    declaration: SourceDeclaration,
    declaration_span: Range<usize>,
    source: &str,
    declarations: &mut Vec<DeclarationMetadata>,
    diagnostics: &mut Vec<ScanDiagnostic>,
) {
    let Some(documentation) = documentation else {
        return;
    };
    let parsed = documentation.parse(source, diagnostics);
    if parsed.description.is_empty() && parsed.metadata.entries.is_empty() {
        return;
    }

    declarations.push(DeclarationMetadata {
        declaration,
        description: parsed.description,
        metadata: parsed.metadata,
        documentation_span: documentation.span,
        declaration_span,
    });
}

fn report_unsupported_metadata(
    documentation: Option<RawDocumentation>,
    source: &str,
    diagnostics: &mut Vec<ScanDiagnostic>,
) {
    let Some(documentation) = documentation else {
        return;
    };
    if documentation.contains_metadata(source) {
        diagnostics.push(ScanDiagnostic {
            span: documentation.span,
            kind: ScanDiagnosticKind::MetadataOnUnsupportedDeclaration,
        });
    }
}

fn skip_function(tokens: &[Token], keyword_index: usize) -> usize {
    let mut index = keyword_index + 1;
    while index < tokens.len() && !tokens[index].is_symbol('{') {
        if tokens[index].is_symbol(';') {
            return index + 1;
        }
        index += 1;
    }
    if index < tokens.len() {
        skip_balanced(tokens, index, '{', '}')
    } else {
        tokens.len()
    }
}

fn skip_attributes(tokens: &[Token], mut index: usize) -> usize {
    while tokens.get(index).is_some_and(|token| token.is_symbol('@')) {
        index += 1;
        if tokens.get(index).is_some_and(|token| token.is_identifier()) {
            index += 1;
        }
        if tokens.get(index).is_some_and(|token| token.is_symbol('(')) {
            index = skip_balanced(tokens, index, '(', ')');
        }
    }
    index
}

fn skip_balanced(tokens: &[Token], open_index: usize, open: char, close: char) -> usize {
    let mut depth = 0_u32;
    let mut index = open_index;
    while index < tokens.len() {
        if tokens[index].is_symbol(open) {
            depth += 1;
        } else if tokens[index].is_symbol(close) {
            depth -= 1;
            if depth == 0 {
                return index + 1;
            }
        }
        index += 1;
    }
    tokens.len()
}

fn skip_to_delimiter(tokens: &[Token], mut index: usize, delimiter: char, end: char) -> usize {
    let mut angle = 0_i32;
    let mut parenthesis = 0_i32;
    let mut bracket = 0_i32;

    while index < tokens.len() {
        let token = &tokens[index];
        if angle == 0
            && parenthesis == 0
            && bracket == 0
            && (token.is_symbol(delimiter) || token.is_symbol(end))
        {
            return index;
        }
        if let TokenKind::Symbol(symbol) = token.kind {
            match symbol {
                '<' => angle += 1,
                '>' => angle = (angle - 1).max(0),
                '(' => parenthesis += 1,
                ')' => parenthesis = (parenthesis - 1).max(0),
                '[' => bracket += 1,
                ']' => bracket = (bracket - 1).max(0),
                _ => {}
            }
        }
        index += 1;
    }
    tokens.len()
}

fn skip_to_semicolon(tokens: &[Token], mut index: usize) -> usize {
    while index < tokens.len() {
        if tokens[index].is_symbol(';') {
            return index + 1;
        }
        if tokens[index].is_symbol('{') {
            return skip_balanced(tokens, index, '{', '}');
        }
        index += 1;
    }
    tokens.len()
}

fn is_identifier(tokens: &[Token], index: usize, source: &str, expected: &str) -> bool {
    tokens
        .get(index)
        .is_some_and(|token| token.is_identifier() && token.text(source) == expected)
}

#[derive(Clone, Debug)]
struct RawDocumentation {
    lines: Vec<Range<usize>>,
    span: Range<usize>,
}

impl RawDocumentation {
    fn normalize(&self, source: &str) -> NormalizedDocumentation {
        let mut text = String::new();
        let mut source_offsets = Vec::new();

        for (line_index, line) in self.lines.iter().enumerate() {
            if line_index == 0 {
                source_offsets.push(line.start);
            } else {
                text.push('\n');
                source_offsets.push(line.start);
            }
            text.push_str(&source[line.clone()]);
            source_offsets.extend(line.start + 1..=line.end);
        }

        NormalizedDocumentation {
            text,
            source_offsets,
        }
    }

    fn contains_metadata(&self, source: &str) -> bool {
        metadata_block_starts(&self.normalize(source).text)
            .next()
            .is_some()
    }

    fn parse(&self, source: &str, diagnostics: &mut Vec<ScanDiagnostic>) -> ParsedDocumentation {
        let normalized = self.normalize(source);
        let mut ranges = Vec::new();
        let mut entries = Vec::new();
        let mut search_from = 0;

        while let Some(relative_start) =
            metadata_block_starts(&normalized.text[search_from..]).next()
        {
            let start = search_from + relative_start;
            match find_metadata_block_end(&normalized.text, start) {
                Some(end) => {
                    let block_source = &normalized.text[start..end];
                    match parse_metadata_block(block_source) {
                        Ok(block) => {
                            entries.extend(block.entries.into_iter().map(|mut entry| {
                                entry.span = normalized
                                    .source_range(start + entry.span.start..start + entry.span.end);
                                entry
                            }));
                        }
                        Err(MetadataParseError {
                            offset, context, ..
                        }) => {
                            let source_offset = normalized.source_offset(start + offset);
                            diagnostics.push(ScanDiagnostic {
                                span: source_offset..source_offset,
                                kind: ScanDiagnosticKind::InvalidMetadata { context },
                            });
                        }
                    }
                    ranges.push(start..end);
                    search_from = end;
                }
                None => {
                    diagnostics.push(ScanDiagnostic {
                        span: normalized.source_range(start..normalized.text.len()),
                        kind: ScanDiagnosticKind::UnterminatedMetadataBlock,
                    });
                    ranges.push(start..normalized.text.len());
                    break;
                }
            }
        }

        ParsedDocumentation {
            description: without_ranges(&normalized.text, &ranges).trim().to_owned(),
            metadata: MetadataBlock { entries },
        }
    }
}

struct ParsedDocumentation {
    description: String,
    metadata: MetadataBlock,
}

struct NormalizedDocumentation {
    text: String,
    /// Maps every normalized byte boundary to a source byte offset.
    source_offsets: Vec<usize>,
}

impl NormalizedDocumentation {
    fn source_offset(&self, normalized_offset: usize) -> usize {
        self.source_offsets[normalized_offset.min(self.source_offsets.len() - 1)]
    }

    fn source_range(&self, normalized_range: Range<usize>) -> Range<usize> {
        self.source_offset(normalized_range.start)..self.source_offset(normalized_range.end)
    }
}

fn metadata_block_starts(text: &str) -> impl Iterator<Item = usize> + '_ {
    let mut line_start = 0;
    std::iter::from_fn(move || {
        while line_start <= text.len() {
            let current = line_start;
            let line_end = text[current..]
                .find('\n')
                .map_or(text.len(), |offset| current + offset);
            line_start = if line_end < text.len() {
                line_end + 1
            } else {
                text.len() + 1
            };
            let line = &text[current..line_end];
            let indentation = line.len() - line.trim_start_matches([' ', '\t']).len();
            if line[indentation..].starts_with("%{") {
                return Some(current + indentation);
            }
        }
        None
    })
}

fn find_metadata_block_end(text: &str, start: usize) -> Option<usize> {
    let mut in_string = false;
    let mut escaped = false;

    for (relative_offset, character) in text[start + 2..].char_indices() {
        let offset = start + 2 + relative_offset;
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
        } else if character == '"' {
            in_string = true;
        } else if character == '}' {
            return Some(offset + 1);
        }
    }
    None
}

fn without_ranges(text: &str, ranges: &[Range<usize>]) -> String {
    let mut output = String::new();
    let mut cursor = 0;
    for range in ranges {
        output.push_str(&text[cursor..range.start]);
        for character in text[range.clone()].chars() {
            if character == '\n' {
                output.push('\n');
            }
        }
        cursor = range.end;
    }
    output.push_str(&text[cursor..]);
    output
}

fn take_documentation(tokens: &[Token], index: usize) -> (Option<RawDocumentation>, usize) {
    let Some(Token {
        kind: TokenKind::DocLine(first_line),
        span: first_span,
    }) = tokens.get(index)
    else {
        return (None, index);
    };

    let mut lines = vec![first_line.clone()];
    let mut span = first_span.clone();
    let mut next = index + 1;
    while let Some(Token {
        kind: TokenKind::DocLine(line),
        span: line_span,
    }) = tokens.get(next)
    {
        lines.push(line.clone());
        span.end = line_span.end;
        next += 1;
    }
    (Some(RawDocumentation { lines, span }), next)
}

#[derive(Clone, Debug)]
struct Token {
    kind: TokenKind,
    span: Range<usize>,
}

impl Token {
    fn is_identifier(&self) -> bool {
        matches!(self.kind, TokenKind::Identifier)
    }

    fn is_symbol(&self, expected: char) -> bool {
        matches!(self.kind, TokenKind::Symbol(symbol) if symbol == expected)
    }

    fn text<'source>(&self, source: &'source str) -> &'source str {
        &source[self.span.clone()]
    }
}

#[derive(Clone, Debug)]
enum TokenKind {
    Identifier,
    Symbol(char),
    DocLine(Range<usize>),
    Barrier,
}

fn lex(source: &str) -> (Vec<Token>, Vec<ScanDiagnostic>) {
    let mut tokens = Vec::new();
    let mut diagnostics = Vec::new();
    let mut index = 0;

    while index < source.len() {
        let remaining = &source[index..];
        if remaining.starts_with("///") {
            let line_end = remaining
                .find(['\n', '\r'])
                .map_or(source.len(), |offset| index + offset);
            let mut content_start = index + 3;
            if source.as_bytes().get(content_start) == Some(&b' ') {
                content_start += 1;
            }
            tokens.push(Token {
                kind: TokenKind::DocLine(content_start..line_end),
                span: index..line_end,
            });
            index = line_end;
        } else if remaining.starts_with("//") {
            let line_end = remaining
                .find(['\n', '\r'])
                .map_or(source.len(), |offset| index + offset);
            tokens.push(Token {
                kind: TokenKind::Barrier,
                span: index..line_end,
            });
            index = line_end;
        } else if remaining.starts_with("/*") {
            let start = index;
            let mut depth = 1_u32;
            index += 2;
            while index < source.len() && depth > 0 {
                if source[index..].starts_with("/*") {
                    depth += 1;
                    index += 2;
                } else if source[index..].starts_with("*/") {
                    depth -= 1;
                    index += 2;
                } else {
                    index += source[index..].chars().next().unwrap().len_utf8();
                }
            }
            if depth > 0 {
                diagnostics.push(ScanDiagnostic {
                    span: start..source.len(),
                    kind: ScanDiagnosticKind::UnterminatedBlockComment,
                });
            }
            tokens.push(Token {
                kind: TokenKind::Barrier,
                span: start..index,
            });
        } else {
            let character = remaining.chars().next().unwrap();
            if character.is_whitespace() {
                index += character.len_utf8();
            } else if character == '#' {
                let line_end = remaining
                    .find(['\n', '\r'])
                    .map_or(source.len(), |offset| index + offset);
                tokens.push(Token {
                    kind: TokenKind::Barrier,
                    span: index..line_end,
                });
                index = line_end;
            } else if character == '_' || is_xid_start(character) {
                let start = index;
                index += character.len_utf8();
                while index < source.len() {
                    let next = source[index..].chars().next().unwrap();
                    if next == '_' || is_xid_continue(next) {
                        index += next.len_utf8();
                    } else {
                        break;
                    }
                }
                tokens.push(Token {
                    kind: TokenKind::Identifier,
                    span: start..index,
                });
            } else {
                let end = index + character.len_utf8();
                tokens.push(Token {
                    kind: TokenKind::Symbol(character),
                    span: index..end,
                });
                index = end;
            }
        }
    }

    (tokens, diagnostics)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MetadataValue;

    fn declaration<'a>(
        metadata: &'a ModuleMetadata,
        expected: &SourceDeclaration,
    ) -> &'a DeclarationMetadata {
        metadata
            .declarations
            .iter()
            .find(|item| &item.declaration == expected)
            .unwrap()
    }

    #[test]
    fn attaches_metadata_to_struct_members_and_global() {
        let source = r#"
            struct Params {
                /// Surface roughness.
                /// %{
                ///   ui.min = 0.0
                ///   ui.max = 1.0
                /// }
                @align(16)
                roughness: f32,

                /// %{ ui.color; ui.space = "linear"; }
                tint: vec4<f32>,
            }

            /// Editable material parameters.
            /// %{
            ///   reflect.parameters
            ///   ui.label = "Material"
            /// }
            @group(3) @binding(0)
            var<uniform> params: Params;
        "#;

        let scanned = scan_module_metadata("root", source);
        assert!(scanned.diagnostics.is_empty(), "{:?}", scanned.diagnostics);

        let roughness = declaration(
            &scanned,
            &SourceDeclaration::StructMember {
                structure: "Params".to_owned(),
                member: "roughness".to_owned(),
            },
        );
        assert_eq!(roughness.description, "Surface roughness.");
        assert_eq!(
            roughness.metadata.get("ui.min"),
            Some(&MetadataValue::Float(0.0))
        );
        assert_eq!(
            roughness.metadata.get("ui.max"),
            Some(&MetadataValue::Float(1.0))
        );

        let tint = declaration(
            &scanned,
            &SourceDeclaration::StructMember {
                structure: "Params".to_owned(),
                member: "tint".to_owned(),
            },
        );
        assert_eq!(
            tint.metadata.get("ui.color"),
            Some(&MetadataValue::Bool(true))
        );

        let params = declaration(
            &scanned,
            &SourceDeclaration::GlobalVariable {
                name: "params".to_owned(),
            },
        );
        assert_eq!(params.description, "Editable material parameters.");
        assert_eq!(
            params.metadata.get("ui.label"),
            Some(&MetadataValue::String("Material".to_owned()))
        );
    }

    #[test]
    fn qualifies_same_named_members_by_parent_struct() {
        let source = r#"
            struct MaterialParams {
                /// %{ ui.color; }
                color: vec4<f32>,
            }
            struct LightParams {
                /// %{ ui.label = "Light color"; }
                color: vec3<f32>,
            }
        "#;

        let scanned = scan_module_metadata("lighting", source);
        assert_eq!(scanned.module, "lighting");
        assert!(scanned.declarations.iter().any(|item| item.declaration
            == SourceDeclaration::StructMember {
                structure: "MaterialParams".to_owned(),
                member: "color".to_owned(),
            }));
        assert!(scanned.declarations.iter().any(|item| item.declaration
            == SourceDeclaration::StructMember {
                structure: "LightParams".to_owned(),
                member: "color".to_owned(),
            }));
    }

    #[test]
    fn scans_overrides_constants_and_unicode_identifiers() {
        let source = r#"
            /// %{ ui.toggle; }
            override ENABLED: bool = true;

            /// Number of iterations.
            /// %{ ui.min = 1; ui.max = 16; }
            const ITERATIONS: u32 = 4u;

            struct 参数 {
                /// %{ ui.label = "曝光"; }
                曝光: f32,
            }
        "#;

        let scanned = scan_module_metadata("配置", source);
        assert!(scanned.diagnostics.is_empty());
        assert!(scanned.declarations.iter().any(|item| item.declaration
            == SourceDeclaration::Override {
                name: "ENABLED".to_owned(),
            }));
        assert!(scanned.declarations.iter().any(|item| item.declaration
            == SourceDeclaration::Constant {
                name: "ITERATIONS".to_owned(),
            }));
        assert!(scanned.declarations.iter().any(|item| item.declaration
            == SourceDeclaration::StructMember {
                structure: "参数".to_owned(),
                member: "曝光".to_owned(),
            }));
    }

    #[test]
    fn ignores_function_locals_and_reports_function_metadata() {
        let source = r#"
            /// %{ ui.label = "Unsupported"; }
            @fragment
            fn fragment() -> @location(0) vec4<f32> {
                /// %{ ui.color; }
                var local: vec4<f32>;
                return local;
            }
        "#;

        let scanned = scan_module_metadata("root", source);
        assert!(scanned.declarations.is_empty());
        assert_eq!(scanned.diagnostics.len(), 1);
        assert_eq!(
            scanned.diagnostics[0].kind,
            ScanDiagnosticKind::MetadataOnUnsupportedDeclaration
        );
    }

    #[test]
    fn reports_invalid_metadata_and_continues() {
        let source = r#"
            struct Params {
                /// %{ ui.label = "bad\q"; }
                broken: f32,

                /// %{ ui.color; }
                valid: vec4<f32>,
            }
        "#;

        let scanned = scan_module_metadata("root", source);
        assert_eq!(scanned.diagnostics.len(), 1);
        assert!(matches!(
            scanned.diagnostics[0].kind,
            ScanDiagnosticKind::InvalidMetadata {
                context: "string escape"
            }
        ));
        assert!(scanned.declarations.iter().any(|item| item.declaration
            == SourceDeclaration::StructMember {
                structure: "Params".to_owned(),
                member: "valid".to_owned(),
            }));
    }

    #[test]
    fn reports_unterminated_metadata_block() {
        let source = r#"
            struct Params {
                /// %{
                ///   ui.color
                value: vec4<f32>,
            }
        "#;

        let scanned = scan_module_metadata("root", source);
        assert_eq!(scanned.diagnostics.len(), 1);
        assert_eq!(
            scanned.diagnostics[0].kind,
            ScanDiagnosticKind::UnterminatedMetadataBlock
        );
    }

    #[test]
    fn remaps_metadata_spans_to_original_source() {
        let source = r#"struct Params {
    /// %{
    ///   ui.color
    /// }
    color: vec4<f32>,
}"#;
        let scanned = scan_module_metadata("root", source);
        let color = declaration(
            &scanned,
            &SourceDeclaration::StructMember {
                structure: "Params".to_owned(),
                member: "color".to_owned(),
            },
        );
        let entry = &color.metadata.entries[0];

        assert_eq!(&source[entry.span.clone()], "ui.color");
        assert_eq!(&source[color.declaration_span.clone()], "color");
    }
}
