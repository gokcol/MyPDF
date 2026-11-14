use mypdf_milestone0::{
    annotations::{self, Annotation, Kind, Style},
    assets, convert, svg,
};

#[test]
fn standard_stamp_text_is_editable_escaped_fitted_and_persistent() {
    let asset = assets::builtin_assets()
        .into_iter()
        .find(|b| b.asset.name.eq_ignore_ascii_case("approved"))
        .unwrap()
        .asset;
    let Kind::Svg { source, .. } = &asset.kind else {
        panic!()
    };
    let fields = svg::text_fields(source).unwrap();
    assert_eq!(fields[0].1, "APPROVED");
    let label = "Reviewed & approved <QA> – İstanbul";
    let edited = svg::replace_text_field(source, 0, label).unwrap();
    assert_eq!(svg::text_fields(&edited).unwrap()[0].1, label);
    assert!(
        edited.contains("textLength="),
        "Long labels must fit the standard frame"
    );
    svg::preview(&edited, Some([0.1, 0.3, 0.8])).unwrap();
    let short = svg::replace_text_field(&edited, 0, "OK").unwrap();
    assert!(
        !short.contains("textLength="),
        "Short labels regain natural spacing"
    );
    let mut a = Annotation::new(
        0,
        [20., 20., 240., 80.],
        Kind::Svg {
            source: edited,
            tint: true,
        },
        Style {
            color: [0.1, 0.3, 0.8],
            ..Default::default()
        },
    );
    a.object.stamp = true;
    let bytes = annotations::export(&convert::blank(400., 400.).unwrap(), &[a.clone()]).unwrap();
    assert_eq!(
        annotations::load(&lopdf::Document::load_mem(&bytes).unwrap()),
        vec![a]
    );
}

#[test]
fn literal_labels_preserve_artwork_and_outlined_text_is_not_faked() {
    let source = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"><path d="M0 0L10 20"/><text x="10" y="20">A &amp; B</text><text x="10" y="60">Other</text></svg>"#;
    let edited = svg::replace_text_field(source, 0, "C < D").unwrap();
    assert!(edited.contains("<path d=\"M0 0L10 20\"/>"));
    assert_eq!(
        svg::text_fields(&edited)
            .unwrap()
            .iter()
            .map(|f| f.1.as_str())
            .collect::<Vec<_>>(),
        ["C < D", "Other"]
    );
    let outlined = r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><path d="M0 0L10 20"/></svg>"#;
    assert!(svg::text_fields(outlined).unwrap().is_empty());
    assert!(svg::replace_text_field(outlined, 0, "Hello").is_err());
}
