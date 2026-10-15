use mypdf_milestone0::{convert, document, header_footer, ocr, redact, viewer, watermark};

fn page_text(bytes: &[u8], page: usize) -> viewer::PageText {
    let pdf = hayro::hayro_syntax::Pdf::new(bytes.to_vec()).unwrap();
    viewer::extract_page(
        &pdf.pages()[page],
        &hayro::hayro_interpret::InterpreterCache::new(),
    )
}
/// The colour at a displayed point of the first page.
fn pixel(bytes: &[u8], x: f32, y: f32) -> [u8; 3] {
    let ([w, _], rgba) = viewer::render_page(bytes, 0, 1.).unwrap();
    let i = (y as usize * w + x as usize) * 4;
    [rgba[i], rgba[i + 1], rgba[i + 2]]
}
/// How many pixels around a displayed point are tinted red.
fn reddish(bytes: &[u8], x: f32, y: f32) -> usize {
    let ([w, _], rgba) = viewer::render_page(bytes, 0, 1.).unwrap();
    let mut n = 0;
    for yy in y as usize - 40..y as usize + 40 {
        for xx in x as usize - 40..x as usize + 40 {
            let i = (yy * w + xx) * 4;
            n += (rgba[i] as i32 - rgba[i + 1] as i32 > 20) as usize;
        }
    }
    n
}
fn png(color: [u8; 4]) -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(60, 30, image::Rgba(color));
    let mut out = std::io::Cursor::new(vec![]);
    image.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

#[test]
fn text_watermarks_cross_every_page_and_come_off_again() {
    let font = document::default_font().unwrap();
    let original = document::fixture(&font).unwrap();
    let (marked, pages) = watermark::apply(&original, &watermark::Watermark::default()).unwrap();
    assert_eq!(pages, 2);
    assert_eq!(watermark::count(&marked).unwrap(), 2);
    // The word is real text, turned 45° counterclockwise: it climbs to the right.
    let text = page_text(&marked, 0);
    let hit = text
        .find("CONFIDENTIAL", 0)
        .into_iter()
        .next()
        .expect("searchable");
    let first = hit.rects.first().unwrap();
    let last = hit.rects.last().unwrap();
    assert!(
        last[0] > first[0] + 100. && last[1] < first[1] - 100.,
        "{first:?} {last:?}"
    );
    // Centred on the page, see-through red over white.
    assert!(reddish(&marked, 297., 421.) > 100);
    assert_eq!(reddish(&original, 297., 421.), 0);
    // Removing it brings the page back.
    let (clean, changed) = watermark::remove(&marked).unwrap();
    assert_eq!(changed, 2);
    assert_eq!(watermark::count(&clean).unwrap(), 0);
    assert!(page_text(&clean, 0).find("CONFIDENTIAL", 0).is_empty());
    assert_eq!(reddish(&clean, 297., 421.), 0);
    assert!(watermark::remove(&clean).is_err(), "nothing left to remove");
}

#[test]
fn image_watermarks_turned_pages_and_page_ranges() {
    let blank = convert::blank(595., 842.).unwrap();
    let mut doc = lopdf::Document::load_mem(&blank).unwrap();
    let page = *doc.get_pages().get(&1).unwrap();
    doc.get_dictionary_mut(page).unwrap().set("Rotate", 90);
    let mut turned = vec![];
    doc.save_to(&mut turned).unwrap();
    let blue = watermark::Watermark {
        mark: watermark::Mark::Image {
            bytes: png([20, 60, 220, 255]),
            width: 0.5,
        },
        opacity: 1.,
        angle: 0.,
        ..Default::default()
    };
    // A turned page is landscape on screen: the picture sits in its middle.
    let (marked, _) = watermark::apply(&turned, &blue).unwrap();
    let [r, g, b] = pixel(&marked, 421., 297.);
    assert!(b > 180 && r < 80 && g < 120, "{:?}", (r, g, b));
    assert_eq!(pixel(&marked, 40., 40.), [255, 255, 255], "only the middle");
    // Pages can be chosen, and a watermark leaves header and footer text alone.
    let font = document::default_font().unwrap();
    let original = document::fixture(&font).unwrap();
    let footer = header_footer::Stamp {
        template: "Footer text".into(),
        ..Default::default()
    };
    let (with_footer, _) = header_footer::apply(&original, &footer).unwrap();
    let second = watermark::Watermark {
        pages: Some((1, 1)),
        ..Default::default()
    };
    let (both, pages) = watermark::apply(&with_footer, &second).unwrap();
    assert_eq!(pages, 1);
    assert_eq!(watermark::count(&both).unwrap(), 1);
    let (back, _) = watermark::remove(&both).unwrap();
    assert!(
        header_footer::summary(&back).unwrap().any(),
        "the footer stays"
    );
    assert!(
        watermark::apply(
            &original,
            &watermark::Watermark {
                opacity: 2.,
                ..Default::default()
            }
        )
        .is_err()
    );
}

#[test]
fn an_invisible_text_layer_lies_over_its_words() {
    let blank = convert::blank(595., 842.).unwrap();
    let lines = vec![ocr::Line {
        text: "Fatura toplamı 1250 EUR".into(),
        rect: [100., 200., 400., 220.],
    }];
    let layered = ocr::add_text_layer(&blank, &[(0, lines)]).unwrap();
    let text = page_text(&layered, 0);
    let hit = text
        .find("toplamı 1250", 0)
        .into_iter()
        .next()
        .expect("searchable");
    for r in &hit.rects {
        assert!(
            r[0] >= 99. && r[2] <= 401. && r[1] >= 195. && r[3] <= 225.,
            "{r:?}"
        );
    }
    // Invisible: the page still looks blank.
    assert_eq!(pixel(&layered, 250., 210.), [255, 255, 255]);
}

#[cfg(target_os = "macos")]
#[test]
fn scanned_pages_become_searchable() {
    let font = document::default_font().unwrap();
    let original = document::fixture(&font).unwrap();
    assert!(
        ocr::recognize(&original, &|_, _| {}).is_err(),
        "pages with text are left alone"
    );
    // Flattening turns the pages into pictures, as a scanner would.
    let (scanned, _) = redact::flatten_document(&original, 200.).unwrap();
    assert!(page_text(&scanned, 0).text.trim().is_empty());
    let seen = std::sync::Mutex::new(vec![]);
    let (searchable, report) = ocr::recognize(&scanned, &|done, total| {
        seen.lock().unwrap().push((done, total))
    })
    .unwrap();
    assert!(report.pages >= 1 && report.lines >= 3, "{report:?}");
    assert_eq!(seen.lock().unwrap().last(), Some(&(2, 2)));
    let text = page_text(&searchable, 0).text.to_lowercase();
    assert!(text.contains("invoice") && text.contains("1250"), "{text}");
}
