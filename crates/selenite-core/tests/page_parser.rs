use std::path::{Path, PathBuf};

use selenite_core::{BlockKind, Page, ParseError, extract_links};

const WELCOME: &str = include_str!("../../../tests/fixtures/vault/Welcome.md");
const RUST: &str = include_str!("../../../tests/fixtures/vault/Projects/Rust.md");
const UNICODE: &str = include_str!("../../../tests/fixtures/vault/Unicode.md");

#[test]
fn parses_fixture_frontmatter_and_markdown_blocks() {
    let page = Page::parse("Welcome.md", WELCOME).unwrap();
    assert_eq!(page.path, PathBuf::from("Welcome.md"));
    assert_eq!(page.title, "Welcome");
    assert_eq!(page.source, WELCOME);
    assert!(page.body.starts_with("# Welcome"));
    assert_eq!(page.frontmatter["published"].as_bool(), Some(false));
    assert_eq!(page.frontmatter["rating"].as_f64(), Some(4.5));
    assert_eq!(page.frontmatter["tags"].as_sequence().unwrap().len(), 2);
    for kind in [
        BlockKind::Heading,
        BlockKind::Paragraph,
        BlockKind::List,
        BlockKind::Item,
        BlockKind::BlockQuote,
        BlockKind::Table,
        BlockKind::ThematicBreak,
        BlockKind::Code,
        BlockKind::Html,
    ] {
        assert!(
            page.blocks.iter().any(|block| block.kind == kind),
            "missing {kind:?}"
        );
    }
    assert!(
        page.blocks
            .iter()
            .any(|block| block.id.as_deref() == Some("intro"))
    );
    for block in &page.blocks {
        assert_eq!(&page.source[block.span.clone()], block.text);
    }
    assert_eq!(
        page.links
            .iter()
            .map(|link| link.target.as_str())
            .collect::<Vec<_>>(),
        vec!["Projects/Rust", "Diagram", "Welcome"]
    );
    assert_eq!(page.links[0].heading.as_deref(), Some("Ownership"));
    assert_eq!(page.links[0].alias.as_deref(), Some("Rust notes"));
    assert!(page.links[1].embedded);
    assert_eq!(page.links[2].heading.as_deref(), Some("^intro"));
}

#[test]
fn parses_nested_path_with_filename_title_and_no_frontmatter() {
    let path = Path::new("Projects").join("Rust.md");
    let page = Page::parse(&path, RUST).unwrap();
    assert_eq!(page.path, path);
    assert_eq!(page.title, "Rust");
    assert!(page.frontmatter.is_empty());
    assert_eq!(page.body, RUST);
    assert_eq!(page.links[0].target, "Welcome");
}

#[test]
fn preserves_unicode_crlf_and_exact_link_spans() {
    let source = UNICODE.replace('\n', "\r\n");
    let page = Page::parse("Unicode.md", &source).unwrap();
    assert_eq!(page.title, "Unicode");
    assert_eq!(page.source, source);
    assert_eq!(
        &source[page.links[0].span.clone()],
        "[[Projects/Rust|所有権]]"
    );
    assert_eq!(&source[page.links[0].target_span.clone()], "Projects/Rust");
    assert_eq!(page.links[1].target, "");
    assert_eq!(page.links[1].heading.as_deref(), Some("^anchor"));
    assert!(page.body.contains("\r\n"));
    assert_eq!(page.links, extract_links(&source).unwrap());
}

#[test]
fn excludes_yaml_code_html_and_escaped_links() {
    let source = "---\r\nlink: '[[metadata]]'\r\n---\r\n`[[inline]]`\n\n    [[indented]]\n\n~~~\n[[fenced]]\n~~~\n\n<!-- [[comment]] -->\n\n\\[[escaped]] <span title=\"[[attribute]]\">[[visible]]</span> ![[ Embed #^block|Alias ]]";
    let links = extract_links(source).unwrap();
    assert_eq!(
        links
            .iter()
            .map(|link| link.target.as_str())
            .collect::<Vec<_>>(),
        vec!["visible", "Embed"]
    );
    let embedded = &links[1];
    assert!(embedded.embedded);
    assert_eq!(&source[embedded.span.clone()], "![[ Embed #^block|Alias ]]");
    assert_eq!(&source[embedded.target_span.clone()], "Embed");
    assert_eq!(embedded.heading.as_deref(), Some("^block"));
    assert_eq!(embedded.alias.as_deref(), Some("Alias "));
}

#[test]
fn malformed_yaml_is_an_error_without_silent_content_loss() {
    assert!(matches!(
        Page::parse("bad.md", "---\ntags: [unfinished\n---\nbody"),
        Err(ParseError::InvalidFrontmatter(_))
    ));
    assert!(matches!(
        Page::parse("bad.md", "---\n- sequence\n---\nbody"),
        Err(ParseError::FrontmatterNotMapping)
    ));
    assert!(matches!(
        Page::parse("bad.md", "---\ntitle: missing end"),
        Err(ParseError::MissingFrontmatterEnd)
    ));
}

#[test]
fn initial_delimiters_are_frontmatter_and_later_delimiters_are_markdown() {
    let page = Page::parse("empty.md", "---\n---\nHello\n\n---\n").unwrap();
    assert!(page.frontmatter.is_empty());
    assert_eq!(page.body, "Hello\n\n---\n");
    assert!(
        page.blocks
            .iter()
            .any(|block| block.kind == BlockKind::ThematicBreak)
    );
    assert!(matches!(
        Page::parse("separator.md", "---\n"),
        Err(ParseError::MissingFrontmatterEnd)
    ));
    assert_eq!(
        Page::parse("bom.MD", "\u{feff}---\nkey: value\n...\ntext")
            .unwrap()
            .body,
        "text"
    );
}

#[test]
fn rejects_absolute_traversing_metadata_and_non_markdown_paths() {
    for path in [
        "",
        "/outside.md",
        "C:\\outside.md",
        "C:outside.md",
        "../outside.md",
        "notes/../../outside.md",
        "notes\\..\\outside.md",
        "./note.md",
        ".selenite/cache.md",
        ".git/note.md",
        "note.txt",
    ] {
        assert!(
            matches!(Page::parse(path, "note"), Err(ParseError::InvalidPath(_))),
            "{path}"
        );
    }
}

#[test]
fn empty_note_and_incomplete_wikilinks_are_preserved() {
    let empty = Page::parse("empty.md", "").unwrap();
    assert!(empty.blocks.is_empty());
    assert!(empty.links.is_empty());
    let source = "[[]] [[#]] [[unfinished\n[[Across\nlines]]\n[[ok]]";
    let page = Page::parse("note.md", source).unwrap();
    assert_eq!(page.source, source);
    assert_eq!(page.links.len(), 1);
    assert_eq!(page.links[0].target, "ok");
}

#[test]
fn tight_list_items_keep_ids_without_claiming_nested_child_markers() {
    let source = "---\r\nkind: tasks\r\n---\r\n- task ☕ ^task\r\n- parent ^parent\r\n  - child ^child\r\n- unmarked parent\r\n  - marked child ^nested\r\n";
    let page = Page::parse("Tasks.md", source).unwrap();
    let addressed: Vec<_> = page
        .blocks
        .iter()
        .filter_map(|block| block.id.as_deref().map(|id| (block.kind, id)))
        .collect();
    assert_eq!(
        addressed,
        vec![
            (BlockKind::Item, "task"),
            (BlockKind::Item, "parent"),
            (BlockKind::Item, "child"),
            (BlockKind::Item, "nested")
        ]
    );
    for block in &page.blocks {
        assert_eq!(block.text, source[block.span.clone()]);
    }
    assert_eq!(page.source, source);
}

#[test]
fn loose_list_paragraphs_own_their_ids_without_parent_duplicates() {
    let page = Page::parse(
        "Loose.md",
        "- first paragraph ^first\n\n- second paragraph ^second\n\n  - nested child ^nested\n",
    )
    .unwrap();
    let addressed: Vec<_> = page
        .blocks
        .iter()
        .filter_map(|block| block.id.as_deref().map(|id| (block.kind, id)))
        .collect();
    assert_eq!(
        addressed,
        vec![
            (BlockKind::Paragraph, "first"),
            (BlockKind::Paragraph, "second"),
            (BlockKind::Item, "nested")
        ]
    );
    assert!(
        !page
            .blocks
            .iter()
            .any(|block| block.kind == BlockKind::List && block.id.is_some())
    );
}

#[test]
fn block_ids_ignore_code_html_links_and_nonterminal_inline_markup() {
    let source = "- `code ^inline`\n- [link ^label](https://example.com)\n- ![image ^image](image.png)\n- <span title=\"^attribute\">text</span>\n- **bold ^bold**\n- \\^escaped\n- plain [link](https://example.com) ^valid\n\n```\n^fenced\n```\n\n<div>\n^html\n</div>\n\n[ref]: https://example.com/^reference\n";
    let page = Page::parse("Markers.md", source).unwrap();
    let addressed: Vec<_> = page
        .blocks
        .iter()
        .filter_map(|block| block.id.as_deref())
        .collect();
    assert_eq!(addressed, ["valid"]);
}
