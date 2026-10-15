use lopdf::{Document, Object, Stream, dictionary};
use mypdf_milestone0::{
    convert,
    form_data::{self, DataFormat},
    form_design::{self, Action, Align, FieldType, Properties, TabOrder},
    form_detect, forms, viewer,
};

fn png() -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(32, 24, image::Rgba([220, 55, 90, 255]));
    let mut out = std::io::Cursor::new(vec![]);
    image.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}
/// Add a field with fresh properties under the next free name.
fn add(bytes: &[u8], kind: FieldType, rect: [f32; 4]) -> (Vec<u8>, lopdf::ObjectId) {
    let name = form_design::next_name(bytes, kind);
    form_design::add(bytes, 0, rect, kind, &Properties::new(kind, &name)).unwrap()
}
fn find(bytes: &[u8], name: &str) -> form_design::Designed {
    form_design::describe(bytes)
        .unwrap()
        .into_iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no field {name}"))
}
fn value(bytes: &[u8], name: &str) -> String {
    forms::fields(bytes)
        .unwrap()
        .into_iter()
        .find(|f| f.name == name)
        .unwrap()
        .value
}
fn fill(bytes: &[u8], name: &str, v: &str) -> Vec<u8> {
    let id = find(bytes, name).id;
    forms::fill(bytes, id, v).unwrap()
}
/// Darkest pixel inside a displayed rectangle of the first page.
fn darkest(bytes: &[u8], r: [f32; 4]) -> u8 {
    let ([w, _], rgba) = viewer::render_page(bytes, 0, 1.).unwrap();
    let mut min = 255;
    for y in r[1] as usize + 2..r[3] as usize - 1 {
        for x in r[0] as usize + 2..r[2] as usize - 1 {
            let i = (y * w + x) * 4;
            min = min.min(rgba[i].min(rgba[i + 1]).min(rgba[i + 2]));
        }
    }
    min
}

/// One of every field type on a blank page.
fn every_field() -> Vec<u8> {
    let mut bytes = convert::blank(595., 842.).unwrap();
    let rects = [
        (FieldType::Text, [60., 60., 260., 82.]),
        (FieldType::Checkbox, [60., 100., 76., 116.]),
        (FieldType::Radio, [60., 130., 76., 146.]),
        (FieldType::Dropdown, [60., 160., 220., 182.]),
        (FieldType::ListBox, [60., 200., 220., 264.]),
        (FieldType::Button, [60., 280., 160., 304.]),
        (FieldType::Image, [300., 60., 420., 150.]),
        (FieldType::Date, [60., 320., 170., 342.]),
        (FieldType::Signature, [300., 200., 500., 254.]),
    ];
    for (kind, rect) in rects {
        bytes = add(&bytes, kind, rect).0;
    }
    // A second radio button joins the first one's group by name.
    let mut second = Properties::new(FieldType::Radio, "Group1");
    second.export_value = "Choice1".into();
    bytes = form_design::add(
        &bytes,
        0,
        [90., 130., 106., 146.],
        FieldType::Radio,
        &second,
    )
    .unwrap()
    .0;
    bytes
}

#[test]
fn every_field_type_is_created_described_and_filled() {
    let bytes = every_field();
    let fields = form_design::describe(&bytes).unwrap();
    let kinds: Vec<(String, FieldType)> = fields.iter().map(|f| (f.name.clone(), f.kind)).collect();
    for (name, kind) in [
        ("Text1", FieldType::Text),
        ("Check Box1", FieldType::Checkbox),
        ("Group1", FieldType::Radio),
        ("Dropdown1", FieldType::Dropdown),
        ("List Box1", FieldType::ListBox),
        ("Button1", FieldType::Button),
        ("Image1", FieldType::Image),
        ("Date1", FieldType::Date),
        ("Signature1", FieldType::Signature),
    ] {
        assert!(kinds.contains(&(name.into(), kind)), "{name} in {kinds:?}");
    }
    let text = find(&bytes, "Text1");
    for (a, b) in text.widgets[0].rect.iter().zip([60., 60., 260., 82.]) {
        assert!((a - b).abs() < 0.01, "{:?}", text.widgets[0].rect);
    }
    // The radio group has two buttons with different export values.
    let group = find(&bytes, "Group1");
    assert_eq!(group.widgets.len(), 2);
    assert_eq!(group.widgets[0].export, "Choice1");
    assert_eq!(group.widgets[1].export, "Choice2");
    // The fill side understands every fillable kind.
    let filled = forms::fields(&bytes).unwrap();
    for f in &filled {
        let expect_reason = f.name == "Button1";
        assert_eq!(
            f.reason.is_some(),
            expect_reason,
            "{} {:?}",
            f.name,
            f.reason
        );
    }
    assert!(
        filled.iter().any(
            |f| matches!(&f.kind, forms::FieldKind::Date { format } if format == "dd.mm.yyyy")
        )
    );
    assert!(
        filled
            .iter()
            .any(|f| matches!(f.kind, forms::FieldKind::Image))
    );
    assert!(
        filled
            .iter()
            .any(|f| matches!(f.kind, forms::FieldKind::Signature { signed: false }))
    );

    let mut bytes = bytes;
    assert!(
        darkest(&bytes, [60., 100., 76., 116.]) > 100,
        "an empty box"
    );
    for (name, v) in [
        ("Text1", "Jane Appleseed"),
        ("Check Box1", "Yes"),
        ("Group1", "Choice2"),
        ("Dropdown1", "Option 2"),
        ("List Box1", "Option 3"),
    ] {
        bytes = fill(&bytes, name, v);
        assert_eq!(value(&bytes, name), v);
    }
    // Dates are checked against the format and written in it.
    let date = find(&bytes, "Date1").id;
    assert!(forms::fill(&bytes, date, "2026-13-01").is_err());
    bytes = forms::fill(&bytes, date, "5.3.2026").unwrap();
    assert_eq!(value(&bytes, "Date1"), "05.03.2026");
    // The values are drawn on the page.
    assert!(darkest(&bytes, [60., 60., 260., 82.]) < 80, "text drawn");
    assert!(darkest(&bytes, [60., 100., 76., 116.]) < 80, "check drawn");
    let d = Document::load_mem(&bytes).unwrap();
    let states: Vec<Vec<u8>> = find(&bytes, "Group1")
        .widgets
        .iter()
        .map(|w| {
            d.get_dictionary(w.id)
                .unwrap()
                .get(b"AS")
                .unwrap()
                .as_name()
                .unwrap()
                .to_vec()
        })
        .collect();
    assert_eq!(states, [b"Off".to_vec(), b"Choice2".to_vec()]);
    // A picture goes into the image field.
    let image = find(&bytes, "Image1").id;
    bytes = form_design::set_image(&bytes, image, &png()).unwrap();
    let ([w, _], rgba) = viewer::render_page(&bytes, 0, 1.).unwrap();
    let i = (105 * w + 360) * 4;
    assert!(rgba[i] > 180 && rgba[i + 1] < 120, "the picture is drawn");
    // Signature fields are signed, not filled.
    assert!(forms::fill(&bytes, find(&bytes, "Signature1").id, "x").is_err());
}

#[test]
fn properties_change_names_looks_choices_and_actions() {
    let bytes = every_field();
    let text = find(&bytes, "Text1");
    let mut p = text.properties.clone();
    p.name = "Full name".into();
    p.tooltip = "As on your passport".into();
    p.required = true;
    p.font_size = 9.;
    p.align = Align::Center;
    p.background = Some([1., 1., 0.8]);
    p.max_length = Some(20);
    p.default_value = "Your name".into();
    let bytes = form_design::update(&bytes, text.id, text.widgets[0].id, &p).unwrap();
    let renamed = find(&bytes, "Full name");
    assert_eq!(renamed.properties, p);
    assert_eq!(value(&bytes, "Full name"), "Your name");
    assert!(
        forms::fields(&bytes)
            .unwrap()
            .iter()
            .any(|f| f.name == "Full name" && f.required)
    );
    // Names stay unique.
    let mut clash = find(&bytes, "Date1").properties;
    clash.name = "Full name".into();
    let date = find(&bytes, "Date1");
    assert!(form_design::update(&bytes, date.id, date.widgets[0].id, &clash).is_err());
    // Choices, a default and typing allowed.
    let dropdown = find(&bytes, "Dropdown1");
    let mut p = dropdown.properties.clone();
    p.options = vec!["Ankara".into(), "İstanbul".into(), "İzmir".into()];
    p.default_value = "İstanbul".into();
    p.editable = true;
    let bytes = form_design::update(&bytes, dropdown.id, dropdown.widgets[0].id, &p).unwrap();
    assert_eq!(find(&bytes, "Dropdown1").properties.options, p.options);
    assert_eq!(value(&bytes, "Dropdown1"), "İstanbul");
    let mut bad = p.clone();
    bad.options = vec![];
    assert!(form_design::update(&bytes, dropdown.id, dropdown.widgets[0].id, &bad).is_err());
    // Buttons: caption and action; links must be web addresses.
    let button = find(&bytes, "Button1");
    let mut p = button.properties.clone();
    p.caption = "Clear".into();
    p.action = Action::ResetForm;
    let bytes = form_design::update(&bytes, button.id, button.widgets[0].id, &p).unwrap();
    assert_eq!(find(&bytes, "Button1").properties.action, Action::ResetForm);
    p.action = Action::OpenLink("javascript:alert(1)".into());
    assert!(form_design::update(&bytes, button.id, button.widgets[0].id, &p).is_err());
    // Read-only fields cannot be filled.
    let check = find(&bytes, "Check Box1");
    let mut p = check.properties.clone();
    p.read_only = true;
    p.checked = true;
    p.export_value = "Agreed".into();
    let bytes = form_design::update(&bytes, check.id, check.widgets[0].id, &p).unwrap();
    assert_eq!(value(&bytes, "Check Box1"), "Agreed");
    assert!(forms::fill(&bytes, check.id, "Off").is_err());
    // Moving and deleting.
    let list = find(&bytes, "List Box1");
    let bytes = form_design::place(&bytes, list.widgets[0].id, [300., 400., 480., 480.]).unwrap();
    assert_eq!(
        find(&bytes, "List Box1").widgets[0].rect,
        [300., 400., 480., 480.]
    );
    let group = find(&bytes, "Group1");
    let bytes = form_design::remove(&bytes, group.id, Some(group.widgets[1].id)).unwrap();
    assert_eq!(find(&bytes, "Group1").widgets.len(), 1);
    let bytes = form_design::remove(&bytes, group.id, None).unwrap();
    assert!(
        form_design::describe(&bytes)
            .unwrap()
            .iter()
            .all(|f| f.name != "Group1")
    );
}

#[test]
fn tab_order_follows_rows_or_columns() {
    let mut bytes = convert::blank(595., 842.).unwrap();
    // Added out of order: bottom right, top left, bottom left, top right.
    for (name, rect) in [
        ("D", [300., 300., 450., 322.]),
        ("A", [60., 100., 210., 122.]),
        ("C", [60., 300., 210., 322.]),
        ("B", [300., 104., 450., 126.]),
    ] {
        bytes = form_design::add(
            &bytes,
            0,
            rect,
            FieldType::Text,
            &Properties::new(FieldType::Text, name),
        )
        .unwrap()
        .0;
    }
    let order = |bytes: &[u8]| -> String {
        let mut fields = form_design::describe(bytes).unwrap();
        fields.sort_by_key(|f| f.widgets[0].tab);
        fields.iter().map(|f| f.name.clone()).collect()
    };
    assert_eq!(order(&bytes), "DACB");
    let rows = form_design::set_tab_order(&bytes, TabOrder::Rows).unwrap();
    assert_eq!(order(&rows), "ABCD");
    let columns = form_design::set_tab_order(&bytes, TabOrder::Columns).unwrap();
    assert_eq!(order(&columns), "ACBD");
    let d = Document::load_mem(&columns).unwrap();
    let page = *d.get_pages().get(&1).unwrap();
    assert_eq!(
        d.get_dictionary(page)
            .unwrap()
            .get(b"Tabs")
            .unwrap()
            .as_name()
            .unwrap(),
        b"C"
    );
}

#[test]
fn flattening_draws_values_and_keeps_signature_fields() {
    let mut bytes = every_field();
    bytes = fill(&bytes, "Text1", "Flattened text");
    bytes = fill(&bytes, "Check Box1", "Yes");
    let (flat, count) = form_design::flatten(&bytes).unwrap();
    assert!(count >= 9, "{count}");
    let left = form_design::describe(&flat).unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].kind, FieldType::Signature);
    // The values are now part of the page.
    let pdf = hayro::hayro_syntax::Pdf::new(flat.clone()).unwrap();
    let text = viewer::extract_page(
        &pdf.pages()[0],
        &hayro::hayro_interpret::InterpreterCache::new(),
    );
    assert!(text.text.contains("Flattened text"), "{}", text.text);
    assert!(
        darkest(&flat, [60., 100., 76., 116.]) < 80,
        "the check stays"
    );
    // With no signature field, the form goes away completely.
    let sig = find(&flat, "Signature1").id;
    let none = form_design::remove(&flat, sig, None).unwrap();
    let (none, _) = form_design::flatten(&none).unwrap();
    assert!(
        Document::load_mem(&none)
            .unwrap()
            .catalog()
            .unwrap()
            .get(b"AcroForm")
            .is_err()
    );
}

#[test]
fn form_data_round_trips_through_fdf_xfdf_and_csv() {
    let mut bytes = every_field();
    for (name, v) in [
        ("Text1", "Çağla Öztürk, \"Ltd\""),
        ("Check Box1", "Yes"),
        ("Group1", "Choice2"),
        ("Dropdown1", "Option 2"),
        ("List Box1", "Option 3"),
        ("Date1", "01.02.2026"),
    ] {
        bytes = fill(&bytes, name, v);
    }
    let (cleared, changed) = form_design::clear(&bytes).unwrap();
    assert_eq!(changed, 6);
    assert_eq!(value(&cleared, "Text1"), "");
    for format in DataFormat::ALL {
        let data = form_data::export(&bytes, format, "Form.pdf").unwrap();
        let (restored, report) = form_data::import(&cleared, &data, format).unwrap();
        assert_eq!(report.filled, 6, "{format:?} {report:?}");
        assert!(
            report.unknown.is_empty() && report.rejected.is_empty(),
            "{report:?}"
        );
        for name in [
            "Text1",
            "Check Box1",
            "Group1",
            "Dropdown1",
            "List Box1",
            "Date1",
        ] {
            assert_eq!(
                value(&restored, name),
                value(&bytes, name),
                "{format:?} {name}"
            );
        }
    }
    // Spreadsheet data: semicolons, words for check boxes, labels for choices, unknown columns.
    let csv = "Text1;Check Box1;Dropdown1;Nickname\r\nAyşe;true;Option 3;Ace\r\n";
    let (filled, report) = form_data::import(&cleared, csv.as_bytes(), DataFormat::Csv).unwrap();
    assert_eq!(report.filled, 3);
    assert_eq!(report.unknown, ["Nickname"]);
    assert_eq!(value(&filled, "Check Box1"), "Yes");
    // Values a field does not accept are reported, not forced in.
    let bad = "Date1,Dropdown1\n31.02.2026,Nope\n";
    let (_, report) = form_data::import(&cleared, bad.as_bytes(), DataFormat::Csv).unwrap();
    assert_eq!(report.filled, 0);
    assert_eq!(report.rejected.len(), 2);
}

#[test]
fn dates_read_and_write_in_acrobat_formats() {
    for (format, text, date, written) in [
        ("dd.mm.yyyy", "5.3.2026", (2026, 3, 5), "05.03.2026"),
        ("mm/dd/yyyy", "12/31/2025", (2025, 12, 31), "12/31/2025"),
        ("yyyy-mm-dd", "2024-02-29", (2024, 2, 29), "2024-02-29"),
        (
            "d mmmm yyyy",
            "7 september 2026",
            (2026, 9, 7),
            "7 September 2026",
        ),
        ("mmm d, yyyy", "Jan 9, 2027", (2027, 1, 9), "Jan 9, 2027"),
    ] {
        assert_eq!(
            form_data::parse_date(format, text).unwrap(),
            date,
            "{format} {text}"
        );
        assert_eq!(form_data::format_date(format, date), written);
    }
    assert!(form_data::parse_date("dd.mm.yyyy", "29.02.2025").is_err());
    assert!(form_data::parse_date("dd.mm.yyyy", "05.03.2026 extra").is_err());
}

/// A page drawn the way paper forms are: blanks, a box, a line and a text area.
fn paper_form() -> Vec<u8> {
    let mut doc = Document::with_version("1.7");
    let pages = doc.new_object_id();
    let font = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding" });
    let content = "BT /F1 12 Tf 72 760 Td (Name: ______________________) Tj ET\n\
        BT /F1 12 Tf 72 730 Td (Date of birth: ______________) Tj ET\n\
        0.5 w 72 700 10 10 re S\n\
        BT /F1 12 Tf 88 701 Td (I agree to the terms) Tj ET\n\
        BT /F1 12 Tf 72 650 Td (Signature) Tj ET\n\
        0.8 w 140 648 m 340 648 l S\n\
        BT /F1 12 Tf 72 610 Td (Comments) Tj ET\n\
        0.8 w 72 500 300 100 re S\n";
    let content = doc.add_object(Stream::new(dictionary! {}, content.as_bytes().to_vec()));
    let page = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages,
        "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
        "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        "Contents" => content,
    });
    doc.objects.insert(
        pages,
        dictionary! { "Type" => "Pages", "Kids" => vec![Object::Reference(page)], "Count" => 1 }
            .into(),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
    doc.trailer.set("Root", catalog);
    let mut out = vec![];
    doc.save_to(&mut out).unwrap();
    out
}

#[test]
fn detection_finds_blanks_boxes_lines_and_text_areas() {
    let bytes = paper_form();
    let found = form_detect::detect(&bytes).unwrap();
    let summary: Vec<(String, FieldType, bool)> = found
        .iter()
        .map(|s| (s.name.clone(), s.kind, s.multiline))
        .collect();
    for expected in [
        ("Name".to_owned(), FieldType::Text, false),
        ("Date of birth".to_owned(), FieldType::Date, false),
        (
            "I agree to the terms".to_owned(),
            FieldType::Checkbox,
            false,
        ),
        ("Signature".to_owned(), FieldType::Signature, false),
        ("Comments".to_owned(), FieldType::Text, true),
    ] {
        assert!(summary.contains(&expected), "{expected:?} in {summary:?}");
    }
    assert_eq!(found.len(), 5, "{summary:?}");
    // The name field sits on the blank, after the label.
    let name = found.iter().find(|s| s.name == "Name").unwrap();
    assert!(
        name.rect[0] > 100. && name.rect[3] > 80. && name.rect[3] < 90.,
        "{:?}",
        name.rect
    );
    let (with_fields, added) = form_detect::add_detected(&bytes, &found).unwrap();
    assert_eq!(added, 5);
    assert_eq!(form_design::describe(&with_fields).unwrap().len(), 5);
    // Detecting again finds nothing new.
    assert!(form_detect::detect(&with_fields).unwrap().is_empty());
}

/// A portrait page turned to landscape (or upside down) with /Rotate.
fn turned_page(degrees: i64) -> Vec<u8> {
    let mut doc = Document::load_mem(&convert::blank(595., 842.).unwrap()).unwrap();
    let page = *doc.get_pages().get(&1).unwrap();
    doc.get_dictionary_mut(page).unwrap().set("Rotate", degrees);
    let mut out = vec![];
    doc.save_to(&mut out).unwrap();
    out
}
/// Whether text reads left to right, upright, on the displayed page.
fn reads_upright(bytes: &[u8], text: &str) -> bool {
    let pdf = hayro::hayro_syntax::Pdf::new(bytes.to_vec()).unwrap();
    let page = viewer::extract_page(
        &pdf.pages()[0],
        &hayro::hayro_interpret::InterpreterCache::new(),
    );
    let Some(hit) = page.find(text, 0).into_iter().next() else {
        return false;
    };
    // Letters advance rightwards along one line.
    let middle = |r: &[f32; 4]| (r[1] + r[3]) / 2.;
    hit.rects
        .windows(2)
        .all(|w| w[1][0] > w[0][0] && (middle(&w[1]) - middle(&w[0])).abs() < 4.)
}

#[test]
fn fields_on_turned_pages_are_placed_and_drawn_upright() {
    for degrees in [90, 180, 270] {
        let mut bytes = turned_page(degrees);
        let (w, h) = if degrees == 180 {
            (595., 842.)
        } else {
            (842., 595.)
        };
        // Every kind of field, at displayed positions on the landscape page.
        let mut placed = vec![];
        for (i, kind) in FieldType::ALL.into_iter().enumerate() {
            let [fw, fh] = kind.default_size();
            let x = 30. + (i % 2) as f32 * 270.;
            let y = 30. + (i / 2) as f32 * 110.;
            assert!(x + fw < w && y + fh < h);
            let rect = [x, y, x + fw, y + fh];
            bytes = add(&bytes, kind, rect).0;
            placed.push((kind, rect));
        }
        let fields = form_design::describe(&bytes).unwrap();
        for (kind, rect) in &placed {
            let f = fields.iter().find(|f| f.kind == *kind).unwrap();
            for (a, b) in f.widgets[0].rect.iter().zip(rect) {
                assert!(
                    (a - b).abs() < 0.05,
                    "{degrees}° {kind:?} {:?} {rect:?}",
                    f.widgets[0].rect
                );
            }
        }
        // Everything stays fillable, and typed text reads the right way up.
        assert!(
            forms::fields(&bytes)
                .unwrap()
                .iter()
                .all(|f| f.reason.is_none() || f.name == "Button1")
        );
        bytes = fill(&bytes, "Text1", "Landscape form");
        bytes = fill(&bytes, "Check Box1", "Yes");
        assert!(darkest(&bytes, placed[0].1) < 80, "{degrees}° text drawn");
        assert!(darkest(&bytes, placed[1].1) < 80, "{degrees}° check drawn");
        // The appearance, drawn into the page, reads left to right.
        let (flat, _) = form_design::flatten(&bytes).unwrap();
        assert!(
            reads_upright(&flat, "Landscape form"),
            "{degrees}° flattened"
        );
        assert!(
            darkest(&flat, placed[0].1) < 80,
            "{degrees}° text stays in place"
        );
    }
}
