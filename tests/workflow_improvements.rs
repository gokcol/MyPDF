use lopdf::{Document, Object};
use mypdf_milestone0::{
    annotations::{self as ann, Annotation, Kind, Session, Style},
    convert,
    navigation_edit::{self as nav, Bookmark, LabelRule, NumberStyle},
    object_tools,
    pages::{self, Operation},
    review,
};
use std::sync::Arc;
fn document() -> Vec<u8> {
    let b = convert::blank(500., 700.).unwrap();
    pages::apply(
        &b,
        &Operation::InsertBlank {
            at: 1,
            width: 500.,
            height: 700.,
        },
    )
    .unwrap()
}
fn marks() -> Vec<Annotation> {
    vec![
        Annotation::new(1, [50., 50., 150., 100.], Kind::Rectangle, Style::default()),
        Annotation::new(
            1,
            [60., 110., 170., 140.],
            Kind::Note {
                text: "Review this section".into(),
            },
            Style::default(),
        ),
    ]
}
#[test]
fn persistent_groups_copy_independently_and_round_trip_with_review_threads() {
    let source = document();
    let mut marks = marks();
    let ids = marks.iter().map(|a| a.id.clone()).collect::<Vec<_>>();
    object_tools::group(&mut marks, &ids, false).unwrap();
    marks[1].object.review = review::Review {
        author: "Reviewer Ö".into(),
        created: review::timestamp(),
        resolved: true,
        replies: vec![review::Reply {
            author: "Colleague".into(),
            created: review::timestamp(),
            text: "Fixed, thanks".into(),
        }],
    };
    let encoded = ann::export(&source, &marks).unwrap();
    let loaded = ann::load(&Document::load_mem(&encoded).unwrap());
    assert_eq!(loaded, marks);
    assert_eq!(object_tools::expand_groups(&loaded, &ids[..1]), ids);
    let copies = object_tools::duplicate(&loaded);
    assert_eq!(copies[0].object.group, copies[1].object.group);
    assert_ne!(copies[0].object.group, marks[0].object.group);
    assert_ne!(copies[0].id, marks[0].id);
    let mut combined = marks.clone();
    combined.extend(copies.clone());
    assert_eq!(
        object_tools::expand_groups(&combined, &[copies[0].id.clone()]),
        copies.iter().map(|a| a.id.clone()).collect::<Vec<_>>()
    );
    let saved = ann::export(&encoded, &combined).unwrap();
    let reopened = ann::load(&Document::load_mem(&saved).unwrap());
    assert_eq!(reopened, combined);
    let mut ungrouped = marks.clone();
    object_tools::group(&mut ungrouped, &ids, true).unwrap();
    assert!(ungrouped.iter().all(|a| a.object.group.is_none()));
}
#[test]
fn history_reveals_changed_page_and_region_through_undo_redo() {
    let mut s = Session::new(Arc::from(document()), vec![], true);
    let mut marks = marks();
    s.commit(marks.clone()).unwrap();
    assert_eq!(s.undo_focus().unwrap().0, 1);
    marks[0].rect = [250., 300., 350., 350.];
    s.commit(marks).unwrap();
    let focus = s.undo_focus().unwrap();
    assert_eq!(focus.0, 1);
    assert!(focus.1.unwrap()[0] <= 50. && focus.1.unwrap()[2] >= 350.);
    s.undo();
    assert_eq!(s.redo_focus(), Some(focus));
    s.redo();
    assert_eq!(s.undo_focus(), Some(focus));
}
#[test]
fn bookmark_edit_preserves_destination_zoom_external_actions_and_unrelated_content() {
    let bytes = document();
    let bookmarks = vec![
        Bookmark {
            source: None,
            level: 0,
            title: "Overview".into(),
            page: Some(0),
        },
        Bookmark {
            source: None,
            level: 1,
            title: "Findings".into(),
            page: Some(1),
        },
    ];
    let bytes = nav::set_bookmarks(&bytes, &bookmarks).unwrap();
    let mut doc = Document::load_mem(&bytes).unwrap();
    let root = doc
        .catalog()
        .unwrap()
        .get(b"Outlines")
        .unwrap()
        .as_reference()
        .unwrap();
    let first = doc
        .get_dictionary(root)
        .unwrap()
        .get(b"First")
        .unwrap()
        .as_reference()
        .unwrap();
    let page = doc.get_pages()[&1];
    let custom = Object::Array(vec![
        Object::Reference(page),
        Object::Name(b"XYZ".to_vec()),
        12.into(),
        650.into(),
        Object::Real(1.25),
    ]);
    doc.get_dictionary_mut(first)
        .unwrap()
        .set("Dest", custom.clone());
    let mut bytes = vec![];
    doc.save_to(&mut bytes).unwrap();
    let mut entries = nav::bookmarks(&bytes).unwrap();
    entries[0].title = "Renamed overview".into();
    let next = nav::set_bookmarks(&bytes, &entries).unwrap();
    let doc = Document::load_mem(&next).unwrap();
    let root = doc
        .catalog()
        .unwrap()
        .get(b"Outlines")
        .unwrap()
        .as_reference()
        .unwrap();
    let first = doc
        .get_dictionary(root)
        .unwrap()
        .get(b"First")
        .unwrap()
        .as_reference()
        .unwrap();
    assert_eq!(
        doc.get_dictionary(first).unwrap().get(b"Dest").unwrap(),
        &custom
    );
    let read = nav::bookmarks(&next).unwrap();
    assert_eq!(read[0].title, "Renamed overview");
    assert_eq!(read[1].level, 1);
    assert_eq!(read[1].page, Some(1));
    assert_eq!(doc.get_pages().len(), 2);
    // Renaming an external link must retain its action, not turn it into a page link.
    let mut external = doc.clone();
    let action = lopdf::dictionary! {"S" => "URI", "URI" => Object::string_literal("https://example.com/reference")};
    external.get_dictionary_mut(first).unwrap().remove(b"Dest");
    external
        .get_dictionary_mut(first)
        .unwrap()
        .set("A", action.clone());
    let mut external_bytes = vec![];
    external.save_to(&mut external_bytes).unwrap();
    let mut external_entries = nav::bookmarks(&external_bytes).unwrap();
    assert_eq!(external_entries[0].page, None);
    external_entries[0].title = "Reference site".into();
    let external_bytes = nav::set_bookmarks(&external_bytes, &external_entries).unwrap();
    let external = Document::load_mem(&external_bytes).unwrap();
    let outline = external
        .catalog()
        .unwrap()
        .get(b"Outlines")
        .unwrap()
        .as_reference()
        .unwrap();
    let bookmark = external
        .get_dictionary(outline)
        .unwrap()
        .get(b"First")
        .unwrap()
        .as_reference()
        .unwrap();
    assert_eq!(
        external
            .get_dictionary(bookmark)
            .unwrap()
            .get(b"A")
            .unwrap()
            .as_dict()
            .unwrap(),
        &action
    );
    let mut invalid = entries.clone();
    invalid[0].level = 2;
    assert!(nav::set_bookmarks(&bytes, &invalid).is_err());
    entries.truncate(1);
    let next = nav::set_bookmarks(&next, &entries).unwrap();
    assert_eq!(nav::bookmarks(&next).unwrap().len(), 1);
}
#[test]
fn labels_use_pdf_numbering_and_survive_annotation_save() {
    let bytes = document();
    let rules = vec![
        LabelRule {
            page: 0,
            style: NumberStyle::None,
            prefix: "Cover".into(),
            start: 1,
        },
        LabelRule {
            page: 1,
            style: NumberStyle::RomanLower,
            prefix: String::new(),
            start: 4,
        },
    ];
    let bytes = nav::set_labels(&bytes, &rules).unwrap();
    assert_eq!(nav::labels(&bytes, 2).unwrap(), ["Cover", "iv"]);
    let bytes = ann::export(&bytes, &marks()).unwrap();
    assert_eq!(nav::label_rules(&bytes).unwrap(), rules);
    assert_eq!(nav::labels(&bytes, 2).unwrap(), ["Cover", "iv"]);
    assert!(nav::set_labels(&bytes, &[rules[0].clone(), rules[0].clone()]).is_err());
    assert_eq!(
        nav::labels(&nav::set_labels(&bytes, &[]).unwrap(), 2).unwrap(),
        ["1", "2"]
    );
}
#[test]
fn export_layout_presets_round_trip_every_setting_and_validate_before_use() {
    let layout = mypdf_milestone0::nup::Layout {
        columns: 3,
        rows: 2,
        margins: [0., 12., 18., 24.],
        trim: [1., 2., 3., 4.],
        header: "Audit".into(),
        footer: "{page}/{pages}".into(),
        watermark: "DRAFT".into(),
        ..Default::default()
    };
    let encoded = serde_json::to_vec(&layout).unwrap();
    let restored: mypdf_milestone0::nup::Layout = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(layout, restored);
    restored.validate().unwrap();
    let mut bad = restored;
    bad.columns = 999;
    assert!(bad.validate().is_err());
}
#[test]
fn annotation_summary_quotes_values_and_never_overwrites_existing_files() {
    let mut marks = marks();
    marks[1].object.name = "=DANGEROUS()".into();
    marks[1].object.review.author = "Review, team".into();
    let report = review::csv(&marks);
    assert!(report.contains("\"2\",\"Note\""));
    assert!(report.contains("\"'=DANGEROUS()\""));
    assert!(report.contains("\"Review, team\""));
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("review.csv");
    mypdf_milestone0::write_new(&path, report.as_bytes()).unwrap();
    assert!(mypdf_milestone0::write_new(&path, b"replace").is_err());
    assert_eq!(std::fs::read(path).unwrap(), report.as_bytes());
}
