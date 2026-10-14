use mypdf_milestone0::header_footer::{self, Kind, Position, Stamp};
use mypdf_milestone0::{annotations, content, document, text_geometry};

fn source() -> Vec<u8> {
    // Two pages; the second is rotated, so placement must follow the displayed page.
    document::fixture(&document::default_font().unwrap()).unwrap()
}

fn texts(bytes: &[u8], page: u32) -> Vec<String> {
    document::text_runs(bytes, page)
        .unwrap()
        .into_iter()
        .map(|r| r.text)
        .collect()
}

fn page_numbers() -> Stamp {
    Stamp {
        kind: Kind::PageNumber,
        template: "Page {page} of {pages}".into(),
        position: Position::BottomRight,
        margins: [20., 12., 30., 30.],
        ..Default::default()
    }
}

#[test]
fn page_numbers_sit_in_the_displayed_corner_of_every_page() {
    let source = source();
    let (bytes, pages) = header_footer::apply(&source, &page_numbers()).unwrap();
    assert_eq!(pages, 2);
    let geometry = annotations::page_geometry(&bytes).unwrap();
    for page in 1..=2u32 {
        let runs = text_geometry::runs(&bytes, page).unwrap();
        let run = runs
            .iter()
            .find(|r| r.text == format!("Page {page} of 2"))
            .unwrap_or_else(|| panic!("page {page}: {:?}", texts(&bytes, page)));
        let [_, _, right, bottom] = run.rect.unwrap();
        let (size, _) = &geometry[page as usize - 1];
        assert!(
            (size[2] - 30. - right).abs() < 3.,
            "right edge {right} on page {page}"
        );
        assert!(
            bottom <= size[3] - 12. + 1. && bottom > size[3] - 30.,
            "bottom edge {bottom} on page {page}"
        );
    }
    // The page's own text is untouched.
    for text in texts(&source, 1) {
        assert!(texts(&bytes, 1).contains(&text), "{text}");
    }
    let summary = header_footer::summary(&bytes).unwrap();
    assert_eq!(summary.page_numbers, 2);
    assert!(summary.any());
    let temp = tempfile::tempdir().unwrap();
    assert!(document::render(&bytes, temp.path(), 1.).is_ok());
}

#[test]
fn ranges_kinds_and_removal() {
    let (numbered, _) = header_footer::apply(&source(), &page_numbers()).unwrap();
    let header = Stamp {
        kind: Kind::Text,
        template: "Gökçöl – 東京 – {file}".into(),
        file: "Contract.pdf".into(),
        pages: Some((1, 1)),
        position: Position::TopCenter,
        ..Default::default()
    };
    let (both, pages) = header_footer::apply(&numbered, &header).unwrap();
    assert_eq!(pages, 1);
    // Fallback fonts split the line into runs; together they read as typed.
    assert!(
        texts(&both, 2)
            .concat()
            .contains("Gökçöl – 東京 – Contract.pdf"),
        "{:?}",
        texts(&both, 2)
    );
    assert!(!texts(&both, 1).iter().any(|t| t.contains("Contract.pdf")));
    let summary = header_footer::summary(&both).unwrap();
    assert_eq!(
        (summary.page_numbers, summary.text, summary.bates),
        (2, 1, 0)
    );

    // Removing page numbers keeps the header text.
    let (unnumbered, changed) = header_footer::remove(&both, &[Kind::PageNumber]).unwrap();
    assert_eq!(changed, 2);
    assert!(!texts(&unnumbered, 1).iter().any(|t| t.starts_with("Page ")));
    assert!(
        texts(&unnumbered, 2)
            .iter()
            .any(|t| t.contains("Contract.pdf"))
    );

    // Removing everything restores the page's own streams.
    let (clean, _) = header_footer::remove(&unnumbered, &Kind::ALL).unwrap();
    assert!(!header_footer::summary(&clean).unwrap().any());
    let original = source();
    for page in 1..=2 {
        assert_eq!(texts(&clean, page), texts(&original, page));
    }
    let streams = |bytes: &[u8]| {
        let doc = lopdf::Document::load_mem(bytes).unwrap();
        let page = *doc.get_pages().get(&1).unwrap();
        doc.get_page_contents(page).len()
    };
    assert_eq!(streams(&clean), streams(&original));
    assert_eq!(header_footer::remove(&clean, &Kind::ALL).unwrap().1, 0);
}

#[test]
fn stamps_are_found_after_another_edit_merges_the_page_content() {
    let (numbered, _) = header_footer::apply(&source(), &page_numbers()).unwrap();
    // Adding text rewrites page 1's content as a single stream.
    let edited = content::add_text(
        &numbered,
        1,
        "Added later",
        [60., 300., 300., 330.],
        14.,
        [0.; 3],
    )
    .unwrap();
    assert_eq!(header_footer::summary(&edited).unwrap().page_numbers, 2);
    let (removed, changed) = header_footer::remove(&edited, &[Kind::PageNumber]).unwrap();
    assert_eq!(changed, 2);
    assert!(texts(&removed, 1).iter().any(|t| t == "Added later"));
    assert!(!texts(&removed, 1).iter().any(|t| t.starts_with("Page ")));
}

#[test]
fn bates_numbers_pad_and_text_that_cannot_fit_is_refused() {
    let bates = Stamp {
        kind: Kind::Bates,
        template: "ACME{bates}".into(),
        start: 41,
        digits: 6,
        position: Position::BottomCenter,
        ..Default::default()
    };
    let (bytes, _) = header_footer::apply(&source(), &bates).unwrap();
    assert!(texts(&bytes, 1).iter().any(|t| t == "ACME000041"));
    assert!(texts(&bytes, 2).iter().any(|t| t == "ACME000042"));

    let wide = Stamp {
        template: "Far too far in".into(),
        margins: [20., 20., 2000., 20.],
        ..Default::default()
    };
    let error = header_footer::apply(&source(), &wide).unwrap_err();
    assert!(format!("{error:#}").contains("does not fit"), "{error:#}");
    let empty = Stamp::default();
    assert!(header_footer::apply(&source(), &empty).is_err());
    let out_of_range = Stamp {
        template: "x".into(),
        pages: Some((1, 5)),
        ..Default::default()
    };
    assert!(header_footer::apply(&source(), &out_of_range).is_err());
}
