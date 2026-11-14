use mypdf_milestone0::{
    svg,
    svg_edit::{self, LayerAction, Scene},
};

const ART: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="10 20 240 120"><defs><linearGradient id="ink"><stop stop-color="#135"/><stop offset="1" stop-color="#79b"/></linearGradient></defs><g transform="translate(30 10) rotate(20) scale(1.4)" fill="#00ff00" font-family="Arial"><rect id="frame" x="10" y="20" width="80" height="40" rx="4" style="fill:url(#ink);stroke:#123456;stroke-width:2"/><text id="label" x="15" y="45" font-size="16">Hello &amp; world</text></g></svg>"##;
fn close(a: f32, b: f32) {
    assert!((a - b).abs() < 0.02, "{a} != {b}");
}
#[test]
fn visual_properties_preserve_definitions_and_override_inline_styles() {
    let scene = Scene::parse(ART).unwrap();
    assert_eq!(scene.elements.len(), 2);
    assert_eq!(scene.elements[1].value("fill", "black"), "#00ff00");
    assert_eq!(scene.elements[1].text.as_ref().unwrap().1, "Hello & world");
    let next = svg_edit::update(
        ART,
        0,
        &[("fill".into(), "#f00".into()), ("rx".into(), "12".into())],
        None,
    )
    .unwrap();
    assert!(next.contains("<defs><linearGradient id=\"ink\">"));
    let scene = Scene::parse(&next).unwrap();
    assert_eq!(scene.elements[0].value("fill", ""), "#f00");
    assert_eq!(scene.elements[0].value("stroke", ""), "#123456");
    assert_eq!(scene.elements[0].value("rx", ""), "12");
    let next = svg_edit::update(
        &next,
        1,
        &[("font-weight".into(), "bold".into())],
        Some("Review <A> & İstanbul"),
    )
    .unwrap();
    assert!(next.contains("Review &lt;A&gt; &amp; İstanbul"));
    svg::preview(&next, None).unwrap();
}
#[test]
fn dragging_and_resizing_respect_viewbox_and_transformed_parent() {
    let scene = Scene::parse(ART).unwrap();
    let (_, geometry) = svg_edit::geometry(ART, &scene).unwrap();
    let before = geometry[0].unwrap();
    let moved = svg_edit::translate(ART, 0, before, 23., -7.).unwrap();
    let (_, geometry) = svg_edit::geometry(&moved, &Scene::parse(&moved).unwrap()).unwrap();
    let after = geometry[0].unwrap();
    close(after.bounds[0], before.bounds[0] + 23.);
    close(after.bounds[1], before.bounds[1] - 7.);
    close(after.bounds[2], before.bounds[2]);
    let scaled = svg_edit::scale(ART, 0, before, [0., 0.], [1.5, 1.5]).unwrap();
    let (_, geometry) = svg_edit::geometry(&scaled, &Scene::parse(&scaled).unwrap()).unwrap();
    let after = geometry[0].unwrap();
    for (a, b) in after.bounds.iter().zip(before.bounds) {
        close(*a, b * 1.5);
    }
}
#[test]
fn object_operations_keep_unique_ids_and_stacking_order() {
    let (duplicated, index) = svg_edit::layer(ART, 0, LayerAction::Duplicate).unwrap();
    let scene = Scene::parse(&duplicated).unwrap();
    assert_eq!(index, 1);
    assert_eq!(scene.elements.len(), 3);
    assert_ne!(
        scene.elements[0].attributes["id"],
        scene.elements[1].attributes["id"]
    );
    assert!(duplicated.contains("url(#ink)"));
    let (raised, index) = svg_edit::layer(&duplicated, 0, LayerAction::Forward).unwrap();
    assert_eq!(index, 1);
    assert_eq!(
        Scene::parse(&raised).unwrap().elements[index].attributes["id"],
        "frame"
    );
    let (deleted, _) = svg_edit::layer(&raised, index, LayerAction::Delete).unwrap();
    assert_eq!(Scene::parse(&deleted).unwrap().elements.len(), 2);
    let (added, index) = svg_edit::add(&deleted, "Ellipse").unwrap();
    assert_eq!(Scene::parse(&added).unwrap().elements[index].tag, "ellipse");
    svg::preview(&added, None).unwrap();
}
#[test]
fn nested_text_and_definition_shapes_are_not_overwritten() {
    let source = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><defs><clipPath id="clip"><rect width="90" height="90"/></clipPath></defs><text y="30"><tspan>Rich</tspan><tspan dx="5">text</tspan></text></svg>"##;
    let scene = Scene::parse(source).unwrap();
    assert_eq!(scene.elements.len(), 1);
    assert!(scene.elements[0].text.is_none());
    assert!(svg_edit::update(source, 0, &[], Some("replacement")).is_err());
    let changed = svg_edit::update(source, 0, &[("fill".into(), "red".into())], None).unwrap();
    assert!(changed.contains("<tspan>Rich</tspan><tspan dx=\"5\">text</tspan>"));
}

#[test]
fn edited_artwork_keeps_vector_source_and_properties_after_pdf_save() {
    use mypdf_milestone0::{
        annotations::{self, Annotation, Kind, Style},
        convert,
    };
    let edited = svg_edit::update(
        ART,
        0,
        &[
            ("rx".into(), "14".into()),
            ("stroke-dasharray".into(), "6 4".into()),
            ("fill-opacity".into(), "0.4".into()),
        ],
        None,
    )
    .unwrap();
    let edited = svg_edit::update(
        &edited,
        1,
        &[
            ("font-family".into(), "Arial".into()),
            ("font-style".into(), "italic".into()),
        ],
        Some("QA & review"),
    )
    .unwrap();
    let mut mark = Annotation::new(
        0,
        [10., 10., 250., 130.],
        Kind::Svg {
            source: edited,
            tint: false,
        },
        Style::default(),
    );
    mark.object.stamp = true;
    let saved = annotations::export(&convert::blank(400., 400.).unwrap(), &[mark.clone()]).unwrap();
    let loaded = annotations::load(&lopdf::Document::load_mem(&saved).unwrap());
    assert_eq!(loaded, vec![mark]);
}

#[test]
fn stamp_border_templates_fit_behind_lettering_and_replace_without_stacking() {
    use svg_edit::{Border, BorderKind};
    let text_only = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="10 20 240 80"><text x="130" y="70" text-anchor="middle" fill="#3d6c25" font-size="28">APPROVED</text></svg>"##;
    let mut options = Border::for_scene(&Scene::parse(text_only).unwrap());
    assert_eq!(options.color, "#3d6c25");
    options.background = Some("#ff9900".into());
    options.background_opacity = 0.25;
    let mut source = text_only.to_owned();
    for kind in BorderKind::ALL {
        options.kind = kind;
        source = svg_edit::set_border(&source, &options, None).unwrap();
        assert_eq!(source.matches("data-mypdf-border=").count(), 1);
        let scene = Scene::parse(&source).unwrap();
        assert_eq!(
            scene.elements.last().unwrap().text.as_ref().unwrap().1,
            "APPROVED"
        );
        let restored = svg_edit::current_border(&source).unwrap().unwrap();
        assert_eq!(restored.kind, kind);
        assert_eq!(restored.background.as_deref(), Some("#ff9900"));
        close(restored.background_opacity, 0.25);
        svg::preview(&source, None).unwrap();
    }
    let restored = svg_edit::remove_border(&source).unwrap();
    assert_eq!(restored, text_only);
}

#[test]
fn replacing_a_selected_frame_preserves_other_shapes_and_text() {
    let mut border = svg_edit::Border::for_scene(&Scene::parse(ART).unwrap());
    border.kind = svg_edit::BorderKind::DoubleRounded;
    let source = svg_edit::set_border(ART, &border, Some(0)).unwrap();
    assert!(!source.contains("id=\"frame\""));
    assert!(source.contains("id=\"ink\""));
    assert!(source.contains("Hello &amp; world"));
    assert_eq!(Scene::parse(&source).unwrap().elements.len(), 3);
    assert!(svg_edit::set_border(ART, &border, Some(1)).is_err());
    border.inset = 1000.;
    assert!(svg_edit::set_border(ART, &border, None).is_err());
}
