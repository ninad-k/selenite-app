use std::path::{Path, PathBuf};

use selenite_core::{BacklinkMap, LinkError, LinkResolver, Page, ResolveError, plan_rename};

fn path(value: &str) -> PathBuf {
    value.split('/').collect()
}

fn page(value: &str, source: &str) -> Page {
    Page::parse(path(value), source).unwrap()
}

fn apply(pages: &[Page], from: &str, to: &str) -> Vec<Page> {
    let plan = plan_rename(pages, &path(from), &path(to)).unwrap();
    pages
        .iter()
        .filter(|page| {
            page.path != path(from) && !plan.pages.iter().any(|changed| changed.path == page.path)
        })
        .cloned()
        .chain(plan.pages.iter().cloned())
        .collect()
}

#[test]
fn resolves_qualified_relative_and_unambiguous_basename_links() {
    let paths = [
        path("Home.md"),
        path("Projects/Rust.md"),
        path("Other/Rust.md"),
        path("Projects/Current.md"),
        path("UPPER.MD"),
    ];
    let resolver = LinkResolver::new(&paths).unwrap();
    let source = path("Projects/Current.md");
    assert_eq!(resolver.resolve(&source, "Home").unwrap(), path("Home.md"));
    assert_eq!(
        resolver.resolve(&source, "../Home.md").unwrap(),
        path("Home.md")
    );
    assert_eq!(
        resolver.resolve(&source, "./Rust").unwrap(),
        path("Projects/Rust.md")
    );
    assert_eq!(
        resolver.resolve(&source, "Other/Rust.md").unwrap(),
        path("Other/Rust.md")
    );
    assert_eq!(
        resolver.resolve(&source, "UPPER.md").unwrap(),
        path("UPPER.MD")
    );
    assert_eq!(resolver.resolve(&source, "").unwrap(), source);
    assert!(
        matches!(resolver.resolve(&source, "Rust"), Err(ResolveError::Ambiguous { candidates, .. }) if candidates.len() == 2)
    );
    assert!(matches!(
        resolver.resolve(&source, "missing"),
        Err(ResolveError::Missing(_))
    ));
    for invalid in [
        "../../escape",
        "a/../Home",
        "C:\\outside",
        ".selenite/cache",
        "./../.git/config",
    ] {
        assert!(
            matches!(
                resolver.resolve(&source, invalid),
                Err(ResolveError::Invalid(_))
            ),
            "{invalid}"
        );
    }
}

#[test]
fn root_page_has_no_priority_over_duplicate_basename() {
    let resolver = LinkResolver::new([path("Topic.md"), path("Notes/Topic.md")]).unwrap();
    assert!(matches!(
        resolver.resolve(Path::new("Topic.md"), "Topic"),
        Err(ResolveError::Ambiguous { .. })
    ));
    assert_eq!(
        resolver.resolve(Path::new("Topic.md"), "./Topic").unwrap(),
        path("Topic.md")
    );
    assert_eq!(
        resolver
            .resolve(&path("Notes/Topic.md"), "../Topic")
            .unwrap(),
        path("Topic.md")
    );
}

#[test]
fn backlinks_only_record_resolved_targets_and_keep_link_spans() {
    let pages = vec![
        page("Z.md", "[[Target]] [[Missing]]"),
        page("Target.md", "[[#^self]]"),
        page("A.md", "![[Target#Heading|alias]] [[Duplicate]]"),
        page("X/Duplicate.md", ""),
        page("Y/Duplicate.md", ""),
    ];
    let map = BacklinkMap::build(&pages).unwrap();
    let backlinks = map.backlinks(Path::new("Target.md"));
    assert_eq!(
        backlinks
            .iter()
            .map(|link| link.source.as_path())
            .collect::<Vec<_>>(),
        vec![Path::new("A.md"), Path::new("Target.md"), Path::new("Z.md")]
    );
    assert!(backlinks[0].link.embedded);
    assert_eq!(backlinks[0].link.heading.as_deref(), Some("Heading"));
    assert_eq!(
        &pages[2].source[backlinks[0].link.span.clone()],
        "![[Target#Heading|alias]]"
    );
    assert!(map.backlinks(&path("X/Duplicate.md")).is_empty());
}

#[test]
fn rename_changes_only_wikilink_target_bytes() {
    let source = "---\r\nrelated: '[[Old]]'\r\n---\r\n☕ ![[ Old #^block|別名 ]] [[Old.md#Heading]] [[#^self]]\r\n`[[Old]]`\r\n\r\n```\r\n[[Old]]\r\n```\r\n\r\n<!-- [[Old]] -->\r\n";
    let pages = vec![page("Old.md", "# Original\r\n"), page("Reader.md", source)];
    let plan = plan_rename(&pages, Path::new("Old.md"), Path::new("New.md")).unwrap();
    assert_eq!(plan.pages.len(), 2);
    let reader = plan
        .pages
        .iter()
        .find(|page| page.path == Path::new("Reader.md"))
        .unwrap();
    let expected =
        source
            .replacen("![[ Old #", "![[ New #", 1)
            .replacen("[[Old.md#", "[[New.md#", 1);
    assert_eq!(reader.source, expected);
    assert_eq!(pages[1].source, source, "planning must not mutate input");
    let renamed = plan
        .pages
        .iter()
        .find(|page| page.path == Path::new("New.md"))
        .unwrap();
    assert_eq!(renamed.title, "New");
    assert_eq!(renamed.source, "# Original\r\n");
}

#[test]
fn rename_does_not_rewrite_markdown_destinations_or_autolinks() {
    let source = "[label](https://example.test/[[Old]])\n\n![image](https://example.test/[[Old]])\n\n[ref]: https://example.test/[[Old]]\n\n[reference][ref]\n\n<https://example.test/[[Old]]>\n\n[[Old]]";
    let pages = vec![page("Old.md", ""), page("Reader.md", source)];
    assert_eq!(pages[1].links.len(), 1);
    let result = apply(&pages, "Old.md", "New.md");
    let reader = result
        .iter()
        .find(|page| page.path == Path::new("Reader.md"))
        .unwrap();
    assert_eq!(
        reader.source,
        source.strip_suffix("[[Old]]").unwrap().to_owned() + "[[New]]"
    );
}

#[test]
fn moving_page_preserves_incoming_outgoing_relative_and_local_links() {
    let pages = vec![
        page(
            "Projects/Old.md",
            "[[./Neighbor]] [[../Home]] [[#^self]] [[Projects/Old#Heading]]",
        ),
        page("Projects/Neighbor.md", "[[./Old.md]]"),
        page("Home.md", "[[Projects/Old]]"),
        page("Archive/Neighbor.md", ""),
    ];
    let result = apply(&pages, "Projects/Old.md", "Archive/New.md");
    let resolver = LinkResolver::new(result.iter().map(|page| &page.path)).unwrap();
    let moved = result
        .iter()
        .find(|page| page.path == path("Archive/New.md"))
        .unwrap();
    assert_eq!(
        moved.source,
        "[[../Projects/Neighbor]] [[../Home]] [[#^self]] [[Archive/New#Heading]]"
    );
    assert_eq!(
        resolver
            .resolve(&moved.path, &moved.links[0].target)
            .unwrap(),
        path("Projects/Neighbor.md")
    );
    let neighbor = result
        .iter()
        .find(|page| page.path == path("Projects/Neighbor.md"))
        .unwrap();
    assert_eq!(neighbor.source, "[[../Archive/New.md]]");
    let backlinks = BacklinkMap::build(&result).unwrap();
    assert_eq!(backlinks.backlinks(&path("Archive/New.md")).len(), 4);
}

#[test]
fn new_basename_collision_qualifies_known_links_and_leaves_ambiguous_ones() {
    let pages = vec![
        page("Old.md", ""),
        page("Topic.md", ""),
        page("Reader.md", "[[Old]] [[Topic]] [[Ambiguous]] [[Missing]]"),
        page("X/Ambiguous.md", ""),
        page("Y/Ambiguous.md", ""),
    ];
    let result = apply(&pages, "Old.md", "Archive/Topic.md");
    let reader = result
        .iter()
        .find(|page| page.path == Path::new("Reader.md"))
        .unwrap();
    assert_eq!(
        reader.source,
        "[[Archive/Topic]] [[./Topic]] [[Ambiguous]] [[Missing]]"
    );
    let resolver = LinkResolver::new(result.iter().map(|page| &page.path)).unwrap();
    assert_eq!(
        resolver
            .resolve(&reader.path, &reader.links[0].target)
            .unwrap(),
        path("Archive/Topic.md")
    );
    assert_eq!(
        resolver
            .resolve(&reader.path, &reader.links[1].target)
            .unwrap(),
        path("Topic.md")
    );
    assert!(matches!(
        resolver.resolve(&reader.path, &reader.links[2].target),
        Err(ResolveError::Ambiguous { .. })
    ));
}

#[test]
fn rename_checks_collision_missing_paths_and_noop() {
    let pages = vec![page("Old.md", ""), page("Taken.md", "")];
    assert!(matches!(
        plan_rename(&pages, Path::new("Old.md"), Path::new("Taken.md")),
        Err(LinkError::DestinationExists(_))
    ));
    assert!(matches!(
        plan_rename(&pages, Path::new("Old.md"), Path::new("taken.md")),
        Err(LinkError::DestinationExists(_))
    ));
    assert!(matches!(
        plan_rename(&pages, Path::new("Missing.md"), Path::new("New.md")),
        Err(LinkError::MissingSource(_))
    ));
    assert!(plan_rename(&pages, Path::new("Old.md"), Path::new("../escape.md")).is_err());
    assert!(
        plan_rename(&pages, Path::new("Old.md"), Path::new("Old.md"))
            .unwrap()
            .pages
            .is_empty()
    );
    assert!(matches!(
        LinkResolver::new([Path::new("Old.md"), Path::new("Old.md")]),
        Err(LinkError::DuplicatePath(_))
    ));
}
