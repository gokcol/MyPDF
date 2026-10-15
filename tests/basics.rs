use lopdf::{Document, Object, Stream, dictionary};
use mypdf_milestone0::{compress, content, convert, document, navigation, security, viewer};

fn page_text(bytes: &[u8]) -> String {
    let pdf = hayro::hayro_syntax::Pdf::new(bytes.to_vec()).unwrap();
    viewer::extract_page(
        &pdf.pages()[0],
        &hayro::hayro_interpret::InterpreterCache::new(),
    )
    .text
}

#[test]
fn passwords_protect_open_and_refuse_signed_pdfs() {
    let font = document::default_font().unwrap();
    let original = document::fixture(&font).unwrap();
    assert!(!security::needs_password(&original));
    let password = "şifre Ğüç 2026";
    let protected = security::protect(&original, password).unwrap();
    assert!(security::needs_password(&protected));
    assert!(hayro::hayro_syntax::Pdf::new(protected.clone()).is_err());
    assert!(security::unlock(&protected, "wrong").is_err());
    let opened = security::unlock(&protected, password).unwrap();
    assert!(!security::needs_password(&opened));
    assert_eq!(page_text(&opened), page_text(&original));
    // Protecting twice, or a signed document, is refused with a reason.
    assert!(security::protect(&protected, "again").is_err());
    assert!(security::protect(&original, "").is_err());
    let mut signed = Document::load_mem(&original).unwrap();
    signed.add_object(dictionary! { "Type" => "Sig", "ByteRange" => vec![0.into(), 1.into(), 2.into(), 3.into()], "Contents" => Object::string_literal("x") });
    let mut signed_bytes = vec![];
    signed.save_to(&mut signed_bytes).unwrap();
    let refused = security::protect(&signed_bytes, "secret")
        .unwrap_err()
        .to_string();
    assert!(refused.contains("signatures"), "{refused}");
}

/// Three pages, an outline with a nested entry and a named destination, and links.
fn navigable() -> Vec<u8> {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let mut kids = vec![];
    for _ in 0..3 {
        let content = doc.add_object(Stream::new(dictionary! {}, vec![]));
        kids.push(doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 600.into(), 800.into()],
            "Contents" => content,
        }));
    }
    let link = |doc: &mut Document, rect: [i64; 4], extra: lopdf::Dictionary| {
        let mut d = dictionary! {
            "Type" => "Annot",
            "Subtype" => "Link",
            "Rect" => rect.iter().map(|v| Object::Integer(*v)).collect::<Vec<_>>(),
        };
        for (k, v) in extra.iter() {
            d.set(k.clone(), v.clone());
        }
        doc.add_object(d)
    };
    let web = link(
        &mut doc,
        [100, 700, 300, 720],
        dictionary! { "A" => dictionary! { "S" => "URI", "URI" => Object::string_literal("https://example.com/terms") } },
    );
    let explicit = link(
        &mut doc,
        [100, 600, 300, 620],
        dictionary! { "Dest" => vec![Object::Reference(kids[2]), "Fit".into()] },
    );
    let named = link(
        &mut doc,
        [100, 500, 300, 520],
        dictionary! { "A" => dictionary! { "S" => "GoTo", "D" => Object::string_literal("appendix") } },
    );
    let next = link(
        &mut doc,
        [100, 400, 300, 420],
        dictionary! { "A" => dictionary! { "S" => "Named", "N" => "NextPage" } },
    );
    let script = link(
        &mut doc,
        [100, 300, 300, 320],
        dictionary! { "A" => dictionary! { "S" => "URI", "URI" => Object::string_literal("javascript:alert(1)") } },
    );
    doc.get_dictionary_mut(kids[0]).unwrap().set(
        "Annots",
        vec![web, explicit, named, next, script]
            .into_iter()
            .map(Object::Reference)
            .collect::<Vec<_>>(),
    );
    doc.objects.insert(
        pages_id,
        dictionary! { "Type" => "Pages", "Kids" => kids.iter().map(|k| Object::Reference(*k)).collect::<Vec<_>>(), "Count" => 3 }.into(),
    );
    // Outline: Introduction (page 1) › Details (page 2), Appendix (named, page 3).
    let outlines = doc.new_object_id();
    let intro = doc.new_object_id();
    let details = doc.new_object_id();
    let appendix = doc.new_object_id();
    doc.objects.insert(details, dictionary! { "Title" => lopdf::text_string("Ayrıntılar"), "Parent" => intro, "Dest" => vec![Object::Reference(kids[1]), "Fit".into()] }.into());
    doc.objects.insert(intro, dictionary! { "Title" => Object::string_literal("Introduction"), "Parent" => outlines, "Next" => appendix, "First" => details, "Last" => details, "Dest" => vec![Object::Reference(kids[0]), "Fit".into()] }.into());
    doc.objects.insert(appendix, dictionary! { "Title" => Object::string_literal("Appendix"), "Parent" => outlines, "Prev" => intro, "A" => dictionary! { "S" => "GoTo", "D" => Object::string_literal("appendix") } }.into());
    doc.objects.insert(
        outlines,
        dictionary! { "Type" => "Outlines", "First" => intro, "Last" => appendix }.into(),
    );
    let dests = doc.add_object(dictionary! { "Names" => vec![Object::string_literal("appendix"), Object::Array(vec![Object::Reference(kids[2]), "Fit".into()])] });
    let catalog = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
        "Outlines" => outlines,
        "Names" => dictionary! { "Dests" => dests },
    });
    doc.trailer.set("Root", catalog);
    let mut out = vec![];
    doc.save_to(&mut out).unwrap();
    out
}

#[test]
fn outline_and_links_lead_to_pages_and_web_addresses() {
    let bytes = navigable();
    let outline = navigation::outline(&bytes).unwrap();
    let summary: Vec<(usize, &str, Option<usize>)> = outline
        .iter()
        .map(|o| (o.level, o.title.as_str(), o.page))
        .collect();
    assert_eq!(
        summary,
        [
            (0, "Introduction", Some(0)),
            (1, "Ayrıntılar", Some(1)),
            (0, "Appendix", Some(2))
        ]
    );
    let links = navigation::links(&bytes).unwrap();
    let targets: Vec<&navigation::Target> = links.iter().map(|l| &l.target).collect();
    assert_eq!(
        targets,
        [
            &navigation::Target::Uri("https://example.com/terms".into()),
            &navigation::Target::Page(2),
            &navigation::Target::Page(2),
            &navigation::Target::Page(1),
        ],
        "scripts are not links"
    );
    // Displayed coordinates run from the top of the page.
    assert_eq!(links[0].rect, [100., 80., 300., 100.]);
    assert!(
        navigation::outline(&convert::blank(595., 842.).unwrap())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn reducing_file_size_shrinks_photos_and_keeps_pages() {
    // A large, detailed picture, as a scan or photo would be.
    let image = image::RgbImage::from_fn(2400, 1800, |x, y| {
        let noise = (x.wrapping_mul(2654435761) ^ y.wrapping_mul(40503)) % 23;
        image::Rgb([
            (x % 256) as u8,
            (y % 256) as u8,
            (((x + y) / 4) % 256) as u8 ^ noise as u8,
        ])
    });
    let mut png = std::io::Cursor::new(vec![]);
    image.write_to(&mut png, image::ImageFormat::Png).unwrap();
    let blank = convert::blank(595., 842.).unwrap();
    let with_photo =
        content::add_image(&blank, 1, [40., 40., 555., 426.], &png.into_inner()).unwrap();
    let images_before = content::images(&with_photo, 1).unwrap();
    for level in compress::Level::ALL {
        let (small, report) = compress::reduce(&with_photo, level).unwrap();
        assert!(
            report.after < report.before && report.images >= 1,
            "{level:?} {report:?}"
        );
        assert_eq!(report.after, small.len());
        let images = content::images(&small, 1).unwrap();
        assert_eq!(images.len(), images_before.len());
        assert!(
            images[0].pixels[0] <= 2340,
            "{level:?} {:?}",
            images[0].pixels
        );
        assert_eq!(
            images[0].rect, images_before[0].rect,
            "the picture keeps its place"
        );
        viewer::render_page(&small, 0, 0.5).unwrap();
    }
    // Smaller levels give smaller files.
    let size = |level| compress::reduce(&with_photo, level).unwrap().0.len();
    assert!(size(compress::Level::Small) < size(compress::Level::High));
}
