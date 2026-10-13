use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use selenite_core::Page;
use selenite_db::{
    Cell, DatabaseSchema, DatabaseSource, DatabaseStore, FieldSchema, Filter, Layout, PropertyType,
    SavedView, Sort,
};

fn scratch() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let root = repository.join(".local").join("tests").join(format!(
        "database-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    root.canonicalize().unwrap()
}

fn write(root: &Path, path: &str, source: &str) {
    let path: PathBuf = path.split('/').collect();
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    assert!(
        path.parent()
            .unwrap()
            .canonicalize()
            .unwrap()
            .starts_with(root)
    );
    fs::write(path, source).unwrap();
}

fn field(name: &str, kind: PropertyType) -> FieldSchema {
    FieldSchema {
        name: name.into(),
        kind,
        required: false,
        options: Vec::new(),
    }
}

fn schema() -> DatabaseSchema {
    let mut fields = vec![
        field("summary", PropertyType::Text),
        field("estimate", PropertyType::Number),
        field("date", PropertyType::Date),
        field("status", PropertyType::Select),
        field("contexts", PropertyType::MultiSelect),
        field("done", PropertyType::Checkbox),
        field("project", PropertyType::Relation),
        field("link", PropertyType::Url),
        field("attachment", PropertyType::File),
    ];
    fields[0].required = true;
    fields[3].options = vec!["todo".into(), "done".into()];
    fields[4].options = vec!["home".into(), "work".into()];
    DatabaseSchema {
        id: "tasks".into(),
        name: "Tasks".into(),
        source: DatabaseSource::Folder {
            path: "Rows".into(),
        },
        fields,
    }
}

fn view() -> SavedView {
    SavedView {
        id: "all".into(),
        name: "All rows".into(),
        database: "tasks".into(),
        filter: None,
        sort: Vec::new(),
        group_by: None,
        layout: Layout::Table,
        calendar_field: None,
        gallery_field: None,
    }
}

fn complete_cells() -> BTreeMap<String, Cell> {
    BTreeMap::from([
        ("summary".into(), Cell::Text("Ship a portable vault".into())),
        ("estimate".into(), Cell::Number(2.5)),
        ("date".into(), Cell::Date("2024-02-29".into())),
        ("status".into(), Cell::Select("todo".into())),
        (
            "contexts".into(),
            Cell::MultiSelect(vec!["home".into(), "work".into()]),
        ),
        ("done".into(), Cell::Checkbox(false)),
        (
            "project".into(),
            Cell::Relation(vec!["[[Project]]".into(), "Archive/Plan.md".into()]),
        ),
        (
            "link".into(),
            Cell::Url("https://example.com/course?id=1".into()),
        ),
        ("attachment".into(), Cell::File("Assets/Photo.png".into())),
    ])
}

#[test]
fn all_property_types_round_trip_through_plain_yaml() {
    let root = scratch();
    let store = DatabaseStore::open(&root).unwrap();
    let expected = complete_cells();
    let row = store
        .create_row(
            &schema(),
            "Rows/Portable.md",
            "# Body\r\n\r\n[[Another]]\r\n",
            &expected,
        )
        .unwrap();
    assert_eq!(row.cells, expected);
    assert_eq!(store.rows(&schema()).unwrap(), vec![row.clone()]);
    let source = fs::read_to_string(root.join("Rows").join("Portable.md")).unwrap();
    assert_eq!(source, row.source);
    assert_eq!(
        Page::parse("Rows/Portable.md", &source).unwrap().body,
        "# Body\r\n\r\n[[Another]]\r\n"
    );
    assert!(source.contains("https://example.com/course?id=1"));
    assert!(!root.join(".selenite").join("cache").exists());
}

#[test]
fn malformed_typed_source_is_reported_without_modifying_it() {
    let invalid = [
        ("summary", "false"),
        ("estimate", "'a lot'"),
        ("estimate", ".nan"),
        ("date", "2025-02-29"),
        ("date", "2024-2-9"),
        ("status", "unknown"),
        ("contexts", "[home, alien]"),
        ("contexts", "[home, home]"),
        ("done", "'false'"),
        ("project", "['[[../escape]]']"),
        ("link", "javascript:alert(1)"),
        ("attachment", "../secret.txt"),
    ];
    for (name, value) in invalid {
        let root = scratch();
        let source = if name == "summary" {
            format!("---\nsummary: {value}\n---\nUnchanged body.")
        } else {
            format!("---\nsummary: Valid\n{name}: {value}\n---\nUnchanged body.")
        };
        write(&root, "Rows/Invalid.md", &source);
        let store = DatabaseStore::open(&root).unwrap();
        let error = store.rows(&schema()).unwrap_err();
        assert!(format!("{error:#}").contains(name), "{name}: {error:#}");
        assert_eq!(
            fs::read_to_string(root.join("Rows").join("Invalid.md")).unwrap(),
            source
        );
    }
}

#[test]
fn folder_membership_uses_components_and_tag_membership_uses_yaml() {
    let root = scratch();
    for (path, tags) in [
        ("Rows/A.md", "[team]"),
        ("Rows/Nested/B.md", "other"),
        ("RowsExtra/C.md", "'#team'"),
    ] {
        write(
            &root,
            path,
            &format!(
                "---\nsummary: member\ntags: {tags}\n---\n#team in the body is not membership."
            ),
        );
    }
    let store = DatabaseStore::open(&root).unwrap();
    let rows = store.rows(&schema()).unwrap();
    assert_eq!(
        rows.iter().map(|row| row.path.as_str()).collect::<Vec<_>>(),
        ["Rows/A.md", "Rows/Nested/B.md"]
    );
    let mut tagged = schema();
    tagged.source = DatabaseSource::Tag { tag: "team".into() };
    let rows = store.rows(&tagged).unwrap();
    assert_eq!(
        rows.iter().map(|row| row.path.as_str()).collect::<Vec<_>>(),
        ["Rows/A.md", "RowsExtra/C.md"]
    );
}

#[test]
fn composable_filters_ranges_and_stable_sort_work_on_persisted_views() {
    let root = scratch();
    for (name, estimate, date, contexts, status) in [
        ("C", 3, "2024-03-02", "[home]", "todo"),
        ("B", 2, "2024-03-01", "[work]", "todo"),
        ("A", 2, "2024-03-01", "[home, work]", "todo"),
        ("Z", 2, "2024-03-01", "[home]", "done"),
    ] {
        write(
            &root,
            &format!("Rows/{name}.md"),
            &format!(
                "---\nsummary: do {name}\nestimate: {estimate}\ndate: '{date}'\ncontexts: {contexts}\nstatus: {status}\n---\nBody"
            ),
        );
    }
    let store = DatabaseStore::open(&root).unwrap();
    store.save_schema(&schema(), None).unwrap();
    let mut saved = view();
    saved.filter = Some(Filter::All {
        filters: vec![
            Filter::Eq {
                field: "status".into(),
                value: Cell::Select("todo".into()),
            },
            Filter::Range {
                field: "estimate".into(),
                min: Some(Cell::Number(2.0)),
                max: Some(Cell::Number(3.0)),
            },
            Filter::Range {
                field: "date".into(),
                min: None,
                max: Some(Cell::Date("2024-03-01".into())),
            },
            Filter::Any {
                filters: vec![
                    Filter::Contains {
                        field: "contexts".into(),
                        value: "home".into(),
                    },
                    Filter::Contains {
                        field: "summary".into(),
                        value: "do B".into(),
                    },
                ],
            },
            Filter::IsEmpty {
                field: "project".into(),
            },
        ],
    });
    saved.sort = vec![Sort {
        field: "estimate".into(),
        descending: true,
    }];
    store.save_view(&saved, None).unwrap();
    let reopened = DatabaseStore::open(&root).unwrap();
    let result = reopened.query_saved("all").unwrap();
    assert_eq!(
        result
            .rows
            .iter()
            .map(|row| row.title.as_str())
            .collect::<Vec<_>>(),
        ["A", "B"]
    );
    assert_eq!(reopened.load_view("all").unwrap(), Some(saved));
    assert_eq!(reopened.list_schemas().unwrap(), vec![schema()]);
    assert_eq!(reopened.list_views().unwrap().len(), 1);
}

#[test]
fn multiselect_groups_keep_sort_order_and_empty_cells_sort_last() {
    let root = scratch();
    write(
        &root,
        "Rows/A.md",
        "---\nsummary: a\nestimate: 1\ncontexts: [work, home]\n---\n",
    );
    write(
        &root,
        "Rows/B.md",
        "---\nsummary: b\nestimate: 2\ncontexts: [home]\n---\n",
    );
    write(&root, "Rows/C.md", "---\nsummary: c\n---\n");
    let mut saved = view();
    saved.layout = Layout::Board;
    saved.group_by = Some("contexts".into());
    saved.sort = vec![Sort {
        field: "estimate".into(),
        descending: true,
    }];
    let result = DatabaseStore::open(root)
        .unwrap()
        .query(&schema(), &saved)
        .unwrap();
    assert_eq!(
        result
            .rows
            .iter()
            .map(|row| row.title.as_str())
            .collect::<Vec<_>>(),
        ["B", "A", "C"]
    );
    assert_eq!(result.groups[0].key, Cell::Select("home".into()));
    assert_eq!(result.groups[0].rows, ["Rows/B.md", "Rows/A.md"]);
    assert_eq!(result.groups[1].key, Cell::Select("work".into()));
    assert_eq!(result.groups[1].rows, ["Rows/A.md"]);
    assert_eq!(result.groups[2].key, Cell::Empty);
    assert_eq!(result.groups[2].rows, ["Rows/C.md"]);
}

#[test]
fn frontmatter_edits_preserve_unknown_values_and_body_and_reject_stale_writes() {
    let root = scratch();
    let source = "\u{feff}---\r\n# a YAML comment\r\nsummary: original\r\nestimate: 4\r\ndone: true\r\nunknown: {nested: [1, 2]}\r\n---\r\n\r\n# Exact body\r\n\r\n`code` and [[Page]] 🪨\r\n";
    write(&root, "Rows/A.md", source);
    let store = DatabaseStore::open(&root).unwrap();
    let changes = BTreeMap::from([
        ("summary".into(), Cell::Text("updated".into())),
        ("estimate".into(), Cell::Number(0.0)),
        ("done".into(), Cell::Checkbox(false)),
    ]);
    let row = store
        .update_row(&schema(), "Rows/A.md", source, &changes)
        .unwrap();
    let before = Page::parse("Rows/A.md", source).unwrap();
    let after = Page::parse("Rows/A.md", &row.source).unwrap();
    assert_eq!(after.body, before.body);
    assert_eq!(after.frontmatter["unknown"], before.frontmatter["unknown"]);
    assert!(row.source.starts_with("\u{feff}---\r\n"));
    assert_eq!(row.cells["estimate"], Cell::Number(0.0));
    assert_eq!(row.cells["done"], Cell::Checkbox(false));
    assert!(!row.cells["done"].is_empty());
    assert!(!row.cells["estimate"].is_empty());
    assert!(
        store
            .update_row(&schema(), "Rows/A.md", source, &changes)
            .is_err()
    );
    assert_eq!(
        fs::read_to_string(root.join("Rows").join("A.md")).unwrap(),
        row.source
    );
    let cleared = store
        .update_row(
            &schema(),
            "Rows/A.md",
            &row.source,
            &BTreeMap::from([("estimate".into(), Cell::Empty)]),
        )
        .unwrap();
    assert_eq!(cleared.cells["estimate"], Cell::Empty);
    assert_eq!(
        Page::parse("Rows/A.md", &cleared.source).unwrap().body,
        before.body
    );
}

#[test]
fn edits_validate_before_writing_and_creation_never_overwrites() {
    let root = scratch();
    let store = DatabaseStore::open(&root).unwrap();
    let row = store
        .create_row(&schema(), "Rows/A.md", "body", &complete_cells())
        .unwrap();
    for changes in [
        BTreeMap::from([("summary".into(), Cell::Empty)]),
        BTreeMap::from([("not_in_schema".into(), Cell::Text("unknown".into()))]),
        BTreeMap::from([("date".into(), Cell::Date("2025-02-29".into()))]),
    ] {
        assert!(
            store
                .update_row(&schema(), "Rows/A.md", &row.source, &changes)
                .is_err()
        );
        assert_eq!(
            fs::read_to_string(root.join("Rows").join("A.md")).unwrap(),
            row.source
        );
    }
    assert!(
        store
            .create_row(&schema(), "Rows/A.md", "replacement", &complete_cells())
            .is_err()
    );
    assert!(
        store
            .create_row(&schema(), "Elsewhere/A.md", "body", &complete_cells())
            .is_err()
    );
    assert!(!root.join("Elsewhere").exists());
    assert_eq!(
        fs::read_to_string(root.join("Rows").join("A.md")).unwrap(),
        row.source
    );
}

#[test]
fn external_markdown_changes_are_seen_without_an_index_or_rebuild() {
    let root = scratch();
    let store = DatabaseStore::open(&root).unwrap();
    store.save_schema(&schema(), None).unwrap();
    store.save_view(&view(), None).unwrap();
    write(&root, "Rows/A.md", "---\nsummary: first\n---\nOld body");
    assert_eq!(
        store.query_saved("all").unwrap().rows[0].cells["summary"],
        Cell::Text("first".into())
    );
    write(
        &root,
        "Rows/A.md",
        "---\nsummary: externally edited\n---\nNew body",
    );
    write(&root, "Rows/B.md", "---\nsummary: added\n---\n");
    let fresh = DatabaseStore::open(&root)
        .unwrap()
        .query_saved("all")
        .unwrap();
    assert_eq!(fresh.rows.len(), 2);
    assert_eq!(
        fresh.rows[0].cells["summary"],
        Cell::Text("externally edited".into())
    );
    assert!(fresh.rows[0].source.ends_with("New body"));
    assert!(!root.join(".selenite").join("cache").exists());
}

#[test]
fn metadata_saves_require_exact_previous_toml_and_reject_unsafe_ids() {
    let root = scratch();
    let store = DatabaseStore::open(&root).unwrap();
    let mut schema = schema();
    store.save_schema(&schema, None).unwrap();
    let before = store.load_schema_document("tasks").unwrap().unwrap();
    schema.name = "Updated name".into();
    assert!(store.save_schema(&schema, None).is_err());
    assert!(store.save_schema(&schema, Some("stale")).is_err());
    store.save_schema(&schema, Some(&before.source)).unwrap();
    assert_eq!(store.load_schema("tasks").unwrap(), Some(schema.clone()));
    store.save_view(&view(), None).unwrap();
    let original = store.load_view_document("all").unwrap().unwrap();
    let mut changed = original.definition;
    changed.name = "Renamed view".into();
    store.save_view(&changed, Some(&original.source)).unwrap();
    assert!(store.save_view(&changed, Some(&original.source)).is_err());
    for id in ["../escape", "a/b", "CON", "LPT1", ""] {
        schema.id = id.into();
        assert!(store.save_schema(&schema, None).is_err(), "{id}");
        assert!(store.load_schema(id).is_err(), "{id}");
    }
}

#[test]
fn invalid_filters_and_layout_fields_fail_before_querying() {
    let schema = schema();
    let mut saved = view();
    for filter in [
        Filter::Eq {
            field: "absent".into(),
            value: Cell::Empty,
        },
        Filter::Eq {
            field: "estimate".into(),
            value: Cell::Text("2".into()),
        },
        Filter::Contains {
            field: "done".into(),
            value: "false".into(),
        },
        Filter::Range {
            field: "estimate".into(),
            min: Some(Cell::Number(3.0)),
            max: Some(Cell::Number(2.0)),
        },
        Filter::Range {
            field: "date".into(),
            min: None,
            max: None,
        },
    ] {
        saved.filter = Some(filter);
        assert!(saved.validate(&schema).is_err());
    }
    saved.filter = None;
    saved.layout = Layout::Board;
    assert!(saved.validate(&schema).is_err());
    saved.layout = Layout::Calendar;
    saved.calendar_field = Some("estimate".into());
    assert!(saved.validate(&schema).is_err());
    saved.calendar_field = Some("date".into());
    saved.validate(&schema).unwrap();
    saved.layout = Layout::Gallery;
    saved.gallery_field = Some("attachment".into());
    saved.validate(&schema).unwrap();
}

#[test]
fn bundled_gtd_is_reusable_configuration_and_installation_preserves_customizations() {
    let root = scratch();
    let store = DatabaseStore::open(&root).unwrap();
    assert_eq!(store.install_gtd_templates().unwrap().len(), 7);
    assert!(store.install_gtd_templates().unwrap().is_empty());
    assert_eq!(store.list_views().unwrap().len(), 6);
    let schema = store.load_schema("gtd").unwrap().unwrap();
    let cells = BTreeMap::from([
        ("status".into(), Cell::Select("next".into())),
        ("kind".into(), Cell::Select("action".into())),
        ("contexts".into(), Cell::MultiSelect(vec!["@home".into()])),
    ]);
    let row = store
        .create_row(
            &schema,
            "Anywhere/Call.md",
            "Call about the project.",
            &cells,
        )
        .unwrap();
    assert!(row.source.contains("gtd"));
    assert_eq!(store.query_saved("next_actions").unwrap().rows, vec![row]);
    assert!(store.query_saved("inbox").unwrap().rows.is_empty());
    let before = store.load_view_document("inbox").unwrap().unwrap();
    let mut custom = before.definition;
    custom.name = "My inbox".into();
    store.save_view(&custom, Some(&before.source)).unwrap();
    assert!(store.install_gtd_templates().unwrap().is_empty());
    assert_eq!(store.load_view("inbox").unwrap().unwrap().name, "My inbox");
}
