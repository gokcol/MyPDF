use mypdf_milestone0::{
    document, identity, signing,
    viewer::{self, Event, PageSize, Worker},
};
use std::time::{Duration, Instant};

fn fixture() -> Vec<u8> {
    document::fixture(&document::default_font().unwrap()).unwrap()
}

#[test]
fn unicode_search_has_geometry_on_cropped_rotated_pages() {
    let pdf = hayro::hayro_syntax::Pdf::new(fixture()).unwrap();
    let cache = hayro::hayro_interpret::InterpreterCache::new();
    let mut bounds = vec![];
    for (i, page) in pdf.pages().iter().enumerate() {
        let text = viewer::extract_page(page, &cache);
        assert!(text.text.contains(document::ORIGINAL), "{}", text.text);
        let hits = text.find("SÖZLEŞME", i);
        assert_eq!(hits.len(), 1, "{}", text.text);
        assert_eq!(&text.text[hits[0].range.clone()], "sözleşme");
        assert!(!hits[0].rects.is_empty());
        let (w, h) = page.render_dimensions();
        for r in &hits[0].rects {
            assert!(r[0] >= 0.0 && r[1] >= 0.0 && r[2] <= w && r[3] <= h);
        }
        let r = hits[0].rects.iter().fold(
            [
                f32::INFINITY,
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::NEG_INFINITY,
            ],
            |a, b| {
                [
                    a[0].min(b[0]),
                    a[1].min(b[1]),
                    a[2].max(b[2]),
                    a[3].max(b[3]),
                ]
            },
        );
        bounds.push(r);
    }
    let portrait = bounds[0];
    let rotated = bounds[1];
    assert!(((portrait[2] - portrait[0]) - (rotated[3] - rotated[1])).abs() < 0.1);
    assert!(((portrait[3] - portrait[1]) - (rotated[2] - rotated[0])).abs() < 0.1);
}

#[test]
fn search_maps_lowercase_expansions_to_original_utf8() {
    let text = viewer::PageText {
        text: "İstanbul Straße Σίσυφος".into(),
        glyphs: vec![],
        ..Default::default()
    };
    let hits = text.find("İSTANBUL", 0);
    assert_eq!(hits.len(), 1);
    assert_eq!(&text.text[hits[0].range.clone()], "İstanbul");
    assert!(text.find("", 0).is_empty());
    assert!(text.find("missing", 0).is_empty());
}

#[test]
fn phrase_search_crosses_pdf_line_breaks_and_normalizes_spaces() {
    let text = viewer::PageText {
        text: "first\n\tsecond   third".into(),
        ..Default::default()
    };
    let hits = text.find(" FIRST second  ", 0);
    assert_eq!(hits.len(), 1);
    assert_eq!(&text.text[hits[0].range.clone()], "first\n\tsecond");
}

#[test]
fn background_search_returns_page_matches_and_identifies_stale_results() {
    let search = viewer::SearchWorker::new(|| {});
    let pages = vec![
        Some(std::sync::Arc::new(viewer::PageText {
            text: "Alpha beta".into(),
            ..Default::default()
        })),
        None,
        Some(std::sync::Arc::new(viewer::PageText {
            text: "Beta gamma".into(),
            ..Default::default()
        })),
    ];
    assert!(search.request(pages.clone(), "alpha".into()));
    let old = search.results.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(old.1[0].page, 0);
    search.invalidate();
    assert_ne!(old.0, search.generation());
    assert!(search.request(pages, "BETA".into()));
    let latest = search.results.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(latest.0, search.generation());
    assert_eq!(latest.1.iter().map(|h| h.page).collect::<Vec<_>>(), [0, 2]);
}

#[test]
fn worker_loads_indexes_and_renders_without_a_window() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("sample.pdf");
    let original = fixture();
    std::fs::write(&path, &original).unwrap();
    let worker = Worker::open(path, || {});
    let deadline = Instant::now() + Duration::from_secs(20);
    let (mut indexed, mut raster) = (0, false);
    while indexed < 2 || !raster {
        assert!(Instant::now() < deadline);
        match worker.events.recv_timeout(Duration::from_secs(5)).unwrap() {
            Event::Loaded {
                bytes,
                pages,
                signed,
                ..
            } => {
                assert_eq!(&*bytes, original);
                assert!(!signed);
                assert_eq!(pages.len(), 2);
                assert!(worker.request(viewer::raster_key(1, pages[1], 0.25, false)));
            }
            Event::Indexed { text, .. } => {
                assert!(!text.find("invoice", 0).is_empty());
                indexed += 1;
            }
            Event::Raster { size, rgba, key } => {
                assert_eq!(key.page, 1);
                assert_eq!(rgba.len(), size[0] * size[1] * 4);
                assert!(rgba.chunks_exact(4).all(|p| p[3] == 255));
                raster = true;
            }
            Event::Failed(e) | Event::RasterFailed { error: e, .. } => panic!("{e}"),
            Event::IndexFinished { limited } => assert!(!limited),
        }
    }
}

#[test]
fn invalid_pdf_is_reported_as_a_recoverable_error() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("invalid.pdf");
    std::fs::write(&path, b"not a PDF").unwrap();
    let worker = Worker::open(path, || {});
    assert!(matches!(
        worker.events.recv_timeout(Duration::from_secs(5)).unwrap(),
        Event::Failed(_)
    ));
}

#[tokio::test]
async fn save_copy_is_exact_and_preserves_signed_revisions() {
    let temp = tempfile::tempdir().unwrap();
    let created = identity::create("Viewer synthetic test", "test-passphrase", true).unwrap();
    let signer = identity::FileSigner::load(&created.pkcs12, "test-passphrase", None).unwrap();
    let signed = signing::sign(&fixture(), &signer, "ViewerSignature", false)
        .await
        .unwrap();
    let destination = temp.path().join("copy.pdf");
    viewer::save_copy(&destination, &signed).unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), signed);
    let report = signing::verify(&std::fs::read(destination).unwrap(), None).unwrap();
    assert!(report[0].crypto_valid && report[0].digest_matches && report[0].byte_range_valid);
}

#[test]
fn save_never_clobbers_an_existing_or_concurrently_created_destination() {
    let temp = tempfile::tempdir().unwrap();
    let destination = temp.path().join("keep.pdf");
    std::fs::write(&destination, b"existing document").unwrap();
    assert!(viewer::save_copy(&destination, &fixture()).is_err());
    assert_eq!(std::fs::read(&destination).unwrap(), b"existing document");
    let concurrent = temp.path().join("race.pdf");
    let a = concurrent.clone();
    let b = concurrent.clone();
    let first = std::thread::spawn(move || viewer::save_copy(&a, b"first"));
    let second = std::thread::spawn(move || viewer::save_copy(&b, b"second"));
    let one = first.join().unwrap().is_ok();
    let two = second.join().unwrap().is_ok();
    assert_ne!(one, two);
    assert_eq!(
        std::fs::read(concurrent).unwrap(),
        if one {
            b"first".as_slice()
        } else {
            b"second".as_slice()
        }
    );
    assert_eq!(
        std::fs::read_dir(temp.path()).unwrap().count(),
        2,
        "Temporary files should be cleaned up"
    );
}

#[cfg(unix)]
#[test]
fn save_does_not_follow_destination_symlinks() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("original.pdf");
    std::fs::write(&target, b"original").unwrap();
    let link = temp.path().join("link.pdf");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(viewer::save_copy(&link, b"replacement").is_err());
    assert_eq!(std::fs::read(target).unwrap(), b"original");
}

#[test]
fn raster_requests_obey_the_memory_and_dimension_budget() {
    let size = PageSize {
        width: 80_000.0,
        height: 70_000.0,
    };
    let key = viewer::raster_key(0, size, 8.0, false);
    // Quantization rounds up; the worker clamps the exact final allocation again.
    assert!(key.factor() < 0.1);
}
