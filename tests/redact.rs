use hayro::{hayro_interpret::InterpreterCache, hayro_syntax::Pdf};
use lopdf::{Document, Object, Stream, dictionary};
use mypdf_milestone0::annotations::{self, Annotation, Kind, Style};
use mypdf_milestone0::redact::{self, Area, Method, Options};
use mypdf_milestone0::{content, convert, document, viewer};

/// The page's text and where a phrase sits on it (union of its glyph boxes).
fn find(bytes: &[u8], page: usize, needle: &str) -> (String, Option<[f32; 4]>) {
    let pdf = Pdf::new(bytes.to_vec()).unwrap();
    let text = viewer::extract_page(&pdf.pages()[page], &InterpreterCache::new());
    let rect = text.find(needle, page).first().map(|hit| {
        hit.rects
            .iter()
            .fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |a, r| {
                [
                    a[0].min(r[0]),
                    a[1].min(r[1]),
                    a[2].max(r[2]),
                    a[3].max(r[3]),
                ]
            })
    });
    (text.text, rect)
}

fn pixel(bytes: &[u8], page: usize, at: [f32; 2]) -> [u8; 3] {
    let ([w, _], rgba) = viewer::render_page(bytes, page, 1.).unwrap();
    let i = (at[1] as usize * w + at[0] as usize) * 4;
    [rgba[i], rgba[i + 1], rgba[i + 2]]
}

fn close(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(a, b)| (a - b).abs() < 0.2)
}

#[test]
fn words_are_removed_from_the_middle_of_a_line_without_moving_the_rest() {
    let page = content::add_text(
        &convert::blank(595., 842.).unwrap(),
        1,
        "Name: Orhan Gökçöl, City: İstanbul",
        [60., 100., 480., 40.],
        16.,
        [0.; 3],
    )
    .unwrap();
    let (_, name) = find(&page, 0, "Orhan Gökçöl");
    let (_, city) = find(&page, 0, "City: İstanbul");
    let (name, city) = (name.unwrap(), city.unwrap());
    let area = [name[0] - 1., name[1] - 2., name[2] + 1., name[3] + 2.];
    let (redacted, report) = redact::apply(
        &page,
        &[Area {
            page: 0,
            rect: area,
        }],
        &Options::default(),
    )
    .unwrap();
    assert_eq!(report.glyphs, "Orhan Gökçöl".chars().count());
    let (text, after) = find(&redacted, 0, "City: İstanbul");
    assert!(
        !text.contains("Orhan") && !text.contains("Gökçöl"),
        "{text}"
    );
    assert!(text.contains("Name:"), "{text}");
    assert!(
        close(after.unwrap(), city),
        "the rest of the line stays put"
    );
    // The box covers the area.
    let middle = [(area[0] + area[2]) / 2., (area[1] + area[3]) / 2.];
    assert_eq!(pixel(&redacted, 0, middle), [0, 0, 0]);
    // The removed words are gone from the file, not just hidden.
    let doc = Document::load_mem(&redacted).unwrap();
    let id = *doc.get_pages().get(&1).unwrap();
    let ops = doc.get_and_decode_page_content(id).unwrap().operations;
    let runs = document::text_runs(&redacted, 1).unwrap();
    assert!(!runs.iter().any(|r| r.text.contains("Orhan")), "{ops:?}");
}

#[test]
fn simple_font_text_images_and_annotations_under_a_box_are_removed() {
    let source = document::fixture(&document::default_font().unwrap()).unwrap();
    let (_, amount) = find(&source, 0, "1250.00");
    let (_, currency) = find(&source, 0, "EUR");
    let amount = amount.unwrap();
    // The checkerboard image, and a mark drawn over it.
    let image = content::images(&source, 1).unwrap()[0].rect;
    let left_half = [
        image[0] - 2.,
        image[1] - 2.,
        (image[0] + image[2]) / 2.,
        image[3] + 2.,
    ];
    let mark = Annotation::new(
        0,
        [
            left_half[0] + 4.,
            left_half[1] + 4.,
            left_half[2] - 4.,
            left_half[3] - 4.,
        ],
        Kind::Rectangle,
        Style::default(),
    );
    // A text box elsewhere on the page keeps its words.
    let kept = Annotation::new(
        0,
        [380., 700., 560., 740.],
        Kind::Text {
            text: "Reviewed".into(),
        },
        Style::default(),
    );
    let marked = annotations::export(&source, &[mark, kept]).unwrap();
    let right = [image[2] - 3., (image[1] + image[3]) / 2.];
    let before_right = pixel(&marked, 0, right);
    let (redacted, report) = redact::apply(
        &marked,
        &[
            Area {
                page: 0,
                rect: [
                    amount[0] - 0.5,
                    amount[1] - 1.,
                    amount[2] + 0.5,
                    amount[3] + 1.,
                ],
            },
            Area {
                page: 0,
                rect: left_half,
            },
        ],
        &Options::default(),
    )
    .unwrap();
    assert_eq!(report.glyphs, 7);
    assert_eq!(report.images, 1);
    assert_eq!(
        report.annotations, 1,
        "the mark over the image goes, the other stays"
    );
    let (text, after) = find(&redacted, 0, "EUR");
    assert!(!text.contains("1250"), "{text}");
    assert!(text.contains("Invoice total:"));
    assert!(close(after.unwrap(), currency.unwrap()));
    assert_eq!(
        pixel(&redacted, 0, [image[0] + 2., image[1] + 2.]),
        [0, 0, 0]
    );
    assert_eq!(
        pixel(&redacted, 0, right),
        before_right,
        "the rest of the image is intact"
    );
    let doc = Document::load_mem(&redacted).unwrap();
    assert_eq!(annotations::load(&doc).len(), 1);

    // White boxes work the same way.
    let (white, _) = redact::apply(
        &source,
        &[Area {
            page: 0,
            rect: left_half,
        }],
        &Options {
            fill: [1.; 3],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        pixel(&white, 0, [image[0] + 2., image[1] + 2.]),
        [255, 255, 255]
    );
}

#[test]
fn rotated_pages_redact_in_displayed_coordinates() {
    let source = document::fixture(&document::default_font().unwrap()).unwrap();
    // Page 2 of the fixture is rotated.
    let (text, split) = find(&source, 1, "Split");
    let split = split.unwrap_or_else(|| panic!("{text}"));
    let (redacted, report) = redact::apply(
        &source,
        &[Area {
            page: 1,
            rect: [split[0] - 1., split[1] - 1., split[2] + 1., split[3] + 1.],
        }],
        &Options::default(),
    )
    .unwrap();
    assert_eq!(report.glyphs, 5);
    let (text, _) = find(&redacted, 1, "text example");
    assert!(
        !text.contains("Split") && text.contains("text example"),
        "{text}"
    );
}

/// A page whose words are drawn by a form XObject.
fn form_page() -> Vec<u8> {
    let mut doc = Document::with_version("1.7");
    let font = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let form = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        },
        b"BT /F1 14 Tf 72 700 Td (Top secret) Tj ET".to_vec(),
    ));
    let content = doc.add_object(Stream::new(dictionary! {}, b"/Fm0 Do".to_vec()));
    let pages = doc.new_object_id();
    let page = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages,
        "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
        "Resources" => dictionary! { "XObject" => dictionary! { "Fm0" => form } },
        "Contents" => content,
    });
    doc.objects.insert(
        pages,
        dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 }.into(),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
    doc.trailer.set("Root", catalog);
    let mut bytes = vec![];
    doc.save_to(&mut bytes).unwrap();
    bytes
}

#[test]
fn text_in_form_objects_is_refused_precisely_and_flattened_on_request() {
    let page = form_page();
    let (text, secret) = find(&page, 0, "Top secret");
    let secret = secret.unwrap_or_else(|| panic!("{text}"));
    let area = [Area {
        page: 0,
        rect: [
            secret[0] - 2.,
            secret[1] - 2.,
            secret[2] + 2.,
            secret[3] + 2.,
        ],
    }];
    let error = redact::apply(&page, &area, &Options::default()).unwrap_err();
    assert!(format!("{error:#}").contains("Flatten"), "{error:#}");
    let (flat, report) = redact::apply(
        &page,
        &area,
        &Options {
            method: Method::Flatten,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(report.flattened, 1);
    let (text, _) = find(&flat, 0, "secret");
    assert!(
        text.trim().is_empty(),
        "flattened pages carry no text: {text}"
    );
    let middle = [(secret[0] + secret[2]) / 2., (secret[1] + secret[3]) / 2.];
    assert_eq!(pixel(&flat, 0, middle), [0, 0, 0]);
    assert!(!String::from_utf8_lossy(&flat).contains("Top secret"));
}

#[test]
fn bad_requests_are_refused() {
    let blank = convert::blank(595., 842.).unwrap();
    assert!(redact::apply(&blank, &[], &Options::default()).is_err());
    let out_of_range = [Area {
        page: 3,
        rect: [0., 0., 10., 10.],
    }];
    assert!(redact::apply(&blank, &out_of_range, &Options::default()).is_err());
    let empty = [Area {
        page: 0,
        rect: [10., 10., 10., 40.],
    }];
    assert!(redact::apply(&blank, &empty, &Options::default()).is_err());
    let _ = Object::Null;
}

#[test]
fn flattening_turns_every_page_into_an_image_with_marks_burned_in() {
    let source = document::fixture(&document::default_font().unwrap()).unwrap();
    let note = Annotation::new(
        0,
        [380., 700., 560., 740.],
        Kind::Rectangle,
        Style {
            fill: Some([0.9, 0.1, 0.1]),
            ..Style::default()
        },
    );
    let marked = annotations::export(&source, &[note]).unwrap();
    let (flat, pages) = redact::flatten_document(&marked, 150.).unwrap();
    assert_eq!(pages, 2);
    assert_eq!(
        annotations::page_geometry(&flat).unwrap(),
        annotations::page_geometry(&source).unwrap(),
        "page sizes and rotation are kept"
    );
    for page in 0..2 {
        let (text, _) = find(&flat, page, "Invoice");
        assert!(text.trim().is_empty(), "page {page} has no text: {text}");
        assert!(
            document::text_runs(&flat, page as u32 + 1)
                .unwrap()
                .is_empty()
        );
    }
    let doc = Document::load_mem(&flat).unwrap();
    assert!(
        annotations::load(&doc).is_empty(),
        "marks are part of the image now"
    );
    let red = pixel(&flat, 0, [470., 720.]);
    assert!(
        red[0] > 180 && red[1] < 120,
        "the mark is burned in: {red:?}"
    );
    assert!(redact::flatten_document(&source, 20.).is_err());
}
