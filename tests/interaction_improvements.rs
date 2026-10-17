use mypdf_milestone0::{
    assets, convert, home,
    rich_text::{self, Span},
    text_geometry, text_style,
    viewer::{PageText, SearchOptions},
};
#[test]
fn search_options_keep_unicode_ranges_and_word_boundaries() {
    let page = PageText {
        text: "Cat cat scatter cat_1 café CAFÉ İstanbul".into(),
        ..Default::default()
    };
    let options = SearchOptions {
        case_sensitive: false,
        whole_word: true,
    };
    assert_eq!(page.find_with("cat", 0, options).len(), 2);
    assert_eq!(
        page.find_with(
            "cat",
            0,
            SearchOptions {
                case_sensitive: true,
                ..options
            }
        )
        .len(),
        1
    );
    let hits = page.find_with("CAFÉ", 0, options);
    assert_eq!(hits.len(), 2);
    assert_eq!(&page.text[hits[0].range.clone()], "café");
    assert_eq!(page.find("cat", 0).len(), 4);
}
#[test]
fn mixed_styles_survive_sequential_edits_wrap_and_pdf_reopen() {
    let fonts = text_style::fonts();
    let normal = &fonts[0];
    let bold = fonts
        .iter()
        .find(|f| f.family == normal.family && f.bold && !f.italic)
        .unwrap();
    let mut text = "Normal bold text in İstanbul".to_owned();
    let mut spans = vec![
        Span {
            range: 0..7,
            font: Some(normal.path.clone()),
            size: 16.,
            color: [0.; 3],
        },
        Span {
            range: 7..text.len(),
            font: Some(bold.path.clone()),
            size: 16.,
            color: [0.7, 0.1, 0.2],
        },
    ];
    for next in [
        "Normal BOLD text in İstanbul",
        "Plain BOLD text in İstanbul",
        "Plain BOLD text in İstanbul\nSecond paragraph",
    ] {
        spans = rich_text::edited(&text, next, &spans);
        text = next.into();
    }
    assert!(spans.len() >= 2);
    assert_eq!(spans[0].font, Some(normal.path.clone()));
    assert_eq!(spans[1].font, Some(bold.path.clone()));
    let pdf = rich_text::add(
        &convert::blank(595., 842.).unwrap(),
        1,
        &text,
        [50., 60., 200., 200.],
        &spans,
    )
    .unwrap();
    let runs = text_geometry::runs(&pdf, 1).unwrap();
    assert!(
        runs.iter()
            .any(|r| r.text.contains("Plain") && r.color == [0.; 3])
    );
    assert!(runs.iter().any(|r| r.text.contains("BOLD")
        && r.base_font.to_lowercase().contains("bold")
        && (r.color[0] - 0.7).abs() < 0.01));
    assert!(runs.iter().all(|r| {
        r.rect
            .is_none_or(|r| r[0] >= 49. && r[2] <= 251. && r[3] <= 261.)
    }));
    assert!(
        rich_text::add(&pdf, 1, &text, [50., 60., 30., 16.], &spans).is_err(),
        "overflow is rejected, never truncated"
    );
}
#[test]
fn standard_stamps_and_custom_metadata_survive_library_reload() {
    let builtins = assets::builtin_assets();
    let stamps: Vec<_> = builtins.iter().filter(|b| b.category == "Stamps").collect();
    assert!(stamps.len() >= 24);
    for b in &stamps {
        b.asset.validate().unwrap();
    }
    for name in [
        "Approved",
        "Draft",
        "NOT APPROVED",
        "FINAL",
        "FOR PUBLIC RELEASE",
        "SIGN HERE",
        "Black check",
    ] {
        assert!(stamps.iter().any(|b| b.asset.name == name), "{name}");
    }
    let dir = tempfile::tempdir().unwrap();
    let mut lib = assets::Library::open(dir.path().into()).unwrap();
    let mut custom = stamps[0].asset.clone();
    custom.id = "cd".repeat(16);
    custom.name = "My approval".into();
    custom.favorite = true;
    custom.folder = "Reviews".into();
    custom.tags = "audit, checked".into();
    custom.last_used = 1234;
    lib.put(custom.clone()).unwrap();
    let restored = assets::Library::open(dir.path().into()).unwrap();
    assert_eq!(restored.entries[0], custom);
    let mut edited = restored.entries[0].clone();
    edited.name = "Revised approval".into();
    lib.put(edited).unwrap();
    assert_eq!(lib.entries.len(), 1);
}
#[test]
fn reading_preferences_survive_other_window_updates_and_do_not_change_pdf() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("read.pdf");
    std::fs::write(&path, b"unchanged document").unwrap();
    let path = path.canonicalize().unwrap();
    let mut a = home::History::load(dir.path().into()).unwrap();
    a.record(&path).unwrap();
    let mut b = home::History::load(dir.path().into()).unwrap();
    let state = home::ReadingState {
        page: 4,
        top: 120.,
        scale: Some(1.5),
        fit_page: false,
        layout: 2,
        sidebar: false,
        inspector: true,
    };
    a.save_reading(&[(path.clone(), state.clone())]).unwrap();
    b.favorite(&path, true).unwrap();
    let restored = home::History::load(dir.path().into()).unwrap();
    assert_eq!(restored.reading(&path), Some(state));
    assert!(restored.entries[0].favorite);
    assert_eq!(std::fs::read(&path).unwrap(), b"unchanged document");
}
