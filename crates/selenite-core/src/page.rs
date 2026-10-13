use std::{
    error::Error,
    fmt,
    ops::Range,
    path::{Path, PathBuf},
};

use pulldown_cmark::{Event, Options, Parser, Tag};

/// A page is always backed by its complete, unchanged Markdown source.
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    /// A vault-relative path. Absolute paths and parent traversal are rejected.
    pub path: PathBuf,
    pub title: String,
    pub frontmatter: serde_yaml::Mapping,
    pub blocks: Vec<Block>,
    pub links: Vec<WikiLink>,
    pub source: String,
    /// Markdown following the frontmatter, with original line endings.
    pub body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    Paragraph,
    Heading,
    BlockQuote,
    Code,
    List,
    Item,
    ThematicBreak,
    Table,
    Html,
    Footnote,
}

/// One Markdown block node; container blocks may contain other block spans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub kind: BlockKind,
    /// UTF-8 byte range in the original page source, including frontmatter.
    pub span: Range<usize>,
    pub text: String,
    pub id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiLink {
    /// Page target, without a heading, alias, or surrounding whitespace.
    /// An empty target denotes a link within the current page.
    pub target: String,
    /// The fragment following `#`, including `^` for a block reference.
    pub heading: Option<String>,
    pub alias: Option<String>,
    pub embedded: bool,
    /// The entire link in the unchanged original source, including `!`.
    pub span: Range<usize>,
    /// Only the trimmed target, so edits preserve fragments and aliases.
    pub target_span: Range<usize>,
}

#[derive(Debug)]
pub enum ParseError {
    InvalidPath(PathBuf),
    MissingFrontmatterEnd,
    InvalidFrontmatter(serde_yaml::Error),
    FrontmatterNotMapping,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath(path) => write!(
                f,
                "invalid vault-relative Markdown path: {}",
                path.display()
            ),
            Self::MissingFrontmatterEnd => {
                f.write_str("YAML frontmatter is missing its closing delimiter")
            }
            Self::InvalidFrontmatter(error) => write!(f, "invalid YAML frontmatter: {error}"),
            Self::FrontmatterNotMapping => f.write_str("YAML frontmatter must be a mapping"),
        }
    }
}

impl Error for ParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidFrontmatter(error) => Some(error),
            _ => None,
        }
    }
}

/// Validate portable vault paths before they are joined to a vault root.
pub fn validate_page_path(path: &Path) -> Result<(), ParseError> {
    if crate::vault::validate_relative(path).is_err() || !crate::vault::is_markdown(path) {
        return Err(ParseError::InvalidPath(path.to_owned()));
    }
    Ok(())
}

impl Page {
    pub fn parse(path: impl AsRef<Path>, source: &str) -> Result<Self, ParseError> {
        let path = path.as_ref();
        validate_page_path(path)?;
        let title = path
            .file_stem()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| ParseError::InvalidPath(path.to_owned()))?
            .to_owned();
        let (frontmatter, body_start) = parse_frontmatter(source)?;
        let body = &source[body_start..];
        let (blocks, excluded) = parse_blocks(body, body_start);
        let links = scan_links(source, body_start, &excluded);
        Ok(Self {
            path: path.to_owned(),
            title,
            frontmatter,
            blocks,
            links,
            source: source.to_owned(),
            body: body.to_owned(),
        })
    }
}

/// Extract links from a complete Markdown source, excluding frontmatter and code.
pub fn extract_links(source: &str) -> Result<Vec<WikiLink>, ParseError> {
    let (_, body_start) = parse_frontmatter(source)?;
    let (_, excluded) = parse_blocks(&source[body_start..], body_start);
    Ok(scan_links(source, body_start, &excluded))
}

fn parse_frontmatter(source: &str) -> Result<(serde_yaml::Mapping, usize), ParseError> {
    let bom = if source.starts_with('\u{feff}') {
        '\u{feff}'.len_utf8()
    } else {
        0
    };
    let mut lines = source[bom..].split_inclusive('\n');
    let Some(first) = lines.next() else {
        return Ok((serde_yaml::Mapping::new(), bom));
    };
    if first.trim_end_matches(['\r', '\n']) != "---" {
        return Ok((serde_yaml::Mapping::new(), bom));
    }
    let start = bom + first.len();
    let mut end = start;
    for line in lines {
        if matches!(line.trim_end_matches(['\r', '\n']), "---" | "...") {
            let yaml = &source[start..end];
            let value: serde_yaml::Value =
                serde_yaml::from_str(yaml).map_err(ParseError::InvalidFrontmatter)?;
            let mapping = match value {
                serde_yaml::Value::Null => serde_yaml::Mapping::new(),
                serde_yaml::Value::Mapping(mapping) => mapping,
                _ => return Err(ParseError::FrontmatterNotMapping),
            };
            return Ok((mapping, end + line.len()));
        }
        end += line.len();
    }
    Err(ParseError::MissingFrontmatterEnd)
}

fn block_kind(tag: &Tag<'_>) -> Option<BlockKind> {
    match tag {
        Tag::Paragraph => Some(BlockKind::Paragraph),
        Tag::Heading { .. } => Some(BlockKind::Heading),
        Tag::BlockQuote(_) => Some(BlockKind::BlockQuote),
        Tag::CodeBlock(_) => Some(BlockKind::Code),
        Tag::List(_) => Some(BlockKind::List),
        Tag::Item => Some(BlockKind::Item),
        Tag::Table(_) => Some(BlockKind::Table),
        Tag::HtmlBlock => Some(BlockKind::Html),
        Tag::FootnoteDefinition(_) => Some(BlockKind::Footnote),
        _ => None,
    }
}

fn parse_blocks(body: &str, offset: usize) -> (Vec<Block>, Vec<Range<usize>>) {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES;
    let mut stack = Vec::new();
    let mut blocks = Vec::new();
    let parser = Parser::new_ext(body, options);
    let mut excluded: Vec<_> = parser
        .reference_definitions()
        .iter()
        .map(|(_, definition)| definition.span.start + offset..definition.span.end + offset)
        .collect();
    for (event, range) in parser.into_offset_iter() {
        match event {
            Event::Start(tag) => {
                let exclude = matches!(
                    tag,
                    Tag::CodeBlock(_) | Tag::HtmlBlock | Tag::Link { .. } | Tag::Image { .. }
                );
                stack.push((block_kind(&tag), range.start, exclude, None));
            }
            Event::End(_) => {
                if let Some((kind, start, exclude, inline_end)) = stack.pop() {
                    let span = start + offset..range.end + offset;
                    if exclude {
                        excluded.push(span.clone());
                    }
                    if let Some(kind) = kind {
                        let mut block = make_block(kind, span, body, offset);
                        let candidate_end = match kind {
                            BlockKind::Paragraph | BlockKind::Heading => Some(range.end),
                            BlockKind::Item => inline_end,
                            _ => None,
                        };
                        block.id = candidate_end
                            .and_then(|end| block_id(&body[start..end], start + offset, &excluded));
                        blocks.push(block);
                    } else {
                        record_item_inline_end(&mut stack, range.end);
                    }
                }
            }
            Event::Code(_) | Event::InlineHtml(_) => {
                excluded.push(range.start + offset..range.end + offset);
                record_item_inline_end(&mut stack, range.end);
            }
            Event::Rule => blocks.push(make_block(
                BlockKind::ThematicBreak,
                range.start + offset..range.end + offset,
                body,
                offset,
            )),
            _ => record_item_inline_end(&mut stack, range.end),
        }
    }
    blocks.sort_by_key(|block| (block.span.start, std::cmp::Reverse(block.span.end)));
    excluded.sort_by_key(|range| range.start);
    (blocks, excluded)
}

fn make_block(kind: BlockKind, span: Range<usize>, body: &str, offset: usize) -> Block {
    let text = body[span.start - offset..span.end - offset].to_owned();
    Block {
        kind,
        span,
        text,
        id: None,
    }
}

// Tight-list items have inline events directly under Item, whereas loose-list
// items own Paragraph nodes. Only the nearest block may claim an inline marker.
fn record_item_inline_end(
    stack: &mut [(Option<BlockKind>, usize, bool, Option<usize>)],
    end: usize,
) {
    if let Some((Some(BlockKind::Item), _, _, inline_end)) =
        stack.iter_mut().rev().find(|(kind, ..)| kind.is_some())
    {
        *inline_end = Some(end);
    }
}

fn block_id(text: &str, offset: usize, excluded: &[Range<usize>]) -> Option<String> {
    let trimmed = text.trim_end();
    let start = trimmed.rfind('^')?;
    if excluded
        .iter()
        .any(|range| range.start < offset + trimmed.len() && range.end > offset + start)
    {
        return None;
    }
    if start > 0 && !trimmed[..start].ends_with(char::is_whitespace) {
        return None;
    }
    let id = &trimmed[start + 1..];
    if id.is_empty()
        || !id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return None;
    }
    Some(id.to_owned())
}

fn escaped(bytes: &[u8], position: usize) -> bool {
    bytes[..position]
        .iter()
        .rev()
        .take_while(|&&byte| byte == b'\\')
        .count()
        % 2
        == 1
}

fn scan_links(source: &str, body_start: usize, excluded: &[Range<usize>]) -> Vec<WikiLink> {
    let bytes = source.as_bytes();
    let mut position = body_start;
    let mut links = Vec::new();
    let mut exclusion = 0;
    while position + 1 < bytes.len() {
        while exclusion < excluded.len() && excluded[exclusion].end <= position {
            exclusion += 1;
        }
        if exclusion < excluded.len() && excluded[exclusion].contains(&position) {
            position = excluded[exclusion].end;
            continue;
        }
        if bytes[position..].starts_with(b"[[") && !escaped(bytes, position) {
            let inner_start = position + 2;
            let Some(relative_end) = source[inner_start..].find("]]") else {
                break;
            };
            let inner_end = inner_start + relative_end;
            let inner = &source[inner_start..inner_end];
            if inner.contains(['\r', '\n'])
                || inner.contains("[[")
                || (exclusion < excluded.len() && excluded[exclusion].start < inner_end + 2)
            {
                position += 2;
                continue;
            }
            let (destination, alias) = match inner.split_once('|') {
                Some((destination, alias)) => (destination, Some(alias.to_owned())),
                None => (inner, None),
            };
            let (target, heading) = match destination.split_once('#') {
                Some((target, heading)) => (target, Some(heading.to_owned())),
                None => (destination, None),
            };
            let trimmed = target.trim();
            if trimmed.is_empty() && heading.as_deref().is_none_or(str::is_empty) {
                position = inner_end + 2;
                continue;
            }
            let target_start = inner_start + (target.len() - target.trim_start().len());
            let embedded = position > body_start
                && bytes[position - 1] == b'!'
                && !escaped(bytes, position - 1);
            links.push(WikiLink {
                target: trimmed.to_owned(),
                heading,
                alias,
                embedded,
                span: position - usize::from(embedded)..inner_end + 2,
                target_span: target_start..target_start + trimmed.len(),
            });
            position = inner_end + 2;
        } else {
            position += 1;
        }
    }
    links
}
