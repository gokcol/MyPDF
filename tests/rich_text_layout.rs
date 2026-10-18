use mypdf_milestone0::{
    convert, document,
    rich_text::{self, LineLayout, Span},
    text_block::{self, Replacement, Selection},
    text_geometry, text_style,
};

fn row(start: usize, left: f32, baseline: f32) -> LineLayout {
    LineLayout {
        start,
        left,
        baseline,
    }
}

fn close(a: f32, b: f32) {
    assert!((a - b).abs() < 0.1, "{a} != {b}");
}

#[test]
fn positioned_replacement_keeps_indents_gaps_and_mixed_styles_after_reopen() {
    let regular = &text_style::fonts()[0];
    let bold = text_style::fonts()
        .iter()
        .find(|font| font.family == regular.family && font.bold && !font.italic)
        .unwrap();
    let text = "Lead BOLD\nIndented\nFinal";
    let spans = vec![
        Span {
            range: 0..5,
            font: Some(regular.path.clone()),
            size: 16.,
            color: [0.; 3],
        },
        Span {
            range: 5..9,
            font: Some(bold.path.clone()),
            size: 22.,
            color: [0.7, 0.1, 0.2],
        },
        Span {
            range: 9..text.len(),
            font: Some(regular.path.clone()),
            size: 16.,
            color: [0.; 3],
        },
    ];
    let layout = vec![row(0, 0., 18.), row(10, 28., 61.), row(19, 10., 107.)];
    let rect = [40., 80., 460., 150.];
    let source = rich_text::add_positioned(
        &convert::blank(595., 842.).unwrap(),
        1,
        text,
        rect,
        &spans,
        &layout,
    )
    .unwrap();
    let before = text_geometry::runs(&source, 1).unwrap();
    let edited = "Lead STRONG\nIndented\nFinal";
    let replacement = Replacement {
        selection: Selection {
            runs: before
                .iter()
                .enumerate()
                .map(|(i, run)| (i, run.text.clone()))
                .collect(),
            annotations: vec![],
        },
        spans: rich_text::edited(text, edited, &spans),
        layout: rich_text::edited_layout(text, edited, &layout, 20.8),
        text: edited.into(),
        rect,
        size: 16.,
        color: [0.; 3],
        format: Default::default(),
    };
    let output = text_block::replace(&source, 1, &replacement).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("preserved-layout.pdf");
    lopdf::Document::load_mem(&output)
        .unwrap()
        .save(&path)
        .unwrap();
    let reopened = std::fs::read(path).unwrap();
    let runs = text_geometry::runs(&reopened, 1).unwrap();
    let find = |text| runs.iter().find(|run| run.text == text).unwrap();
    let lead = find("Lead ");
    let strong = find("STRONG");
    let indented = find("Indented");
    let final_line = find("Final");
    close(lead.layout.as_ref().unwrap().origin[1], 98.);
    close(strong.layout.as_ref().unwrap().origin[1], 98.);
    close(strong.size, 22.);
    assert!(strong.base_font.to_ascii_lowercase().contains("bold"));
    close(strong.color[0], 0.7);
    for (run, x, y) in [(indented, 68., 141.), (final_line, 50., 187.)] {
        let origin = run.layout.as_ref().unwrap().origin;
        close(origin[0], x);
        close(origin[1], y);
        let original = before.iter().find(|old| old.text == run.text).unwrap();
        for (a, b) in original.rect.unwrap().into_iter().zip(run.rect.unwrap()) {
            close(a, b);
        }
    }
    assert!(
        !document::text_runs(&reopened, 1)
            .unwrap()
            .iter()
            .any(|r| r.text == "BOLD")
    );
}

#[test]
fn positioned_soft_wraps_keep_indent_and_shift_following_original_baselines() {
    let text = "Alpha beta gamma delta epsilon zeta eta theta\nTail";
    let spans = vec![Span {
        range: 0..text.len(),
        font: None,
        size: 14.,
        color: [0.; 3],
    }];
    let layout = vec![
        row(0, 20., 18.),
        row(text.find('\n').unwrap() + 1, 40., 80.),
    ];
    let output = rich_text::add_positioned(
        &convert::blank(595., 842.).unwrap(),
        1,
        text,
        [40., 60., 150., 250.],
        &spans,
        &layout,
    )
    .unwrap();
    let runs = text_geometry::runs(&output, 1).unwrap();
    let tail = runs.iter().find(|run| run.text == "Tail").unwrap();
    let wrapped: Vec<_> = runs
        .iter()
        .filter(|run| run.text != "Tail" && !run.text.is_empty())
        .collect();
    assert!(wrapped.len() > 1);
    for (index, run) in wrapped.iter().enumerate() {
        let origin = run.layout.as_ref().unwrap().origin;
        close(origin[0], 60.);
        close(origin[1], 78. + index as f32 * 18.2);
    }
    let tail_origin = tail.layout.as_ref().unwrap().origin;
    close(tail_origin[0], 80.);
    close(tail_origin[1], 140. + (wrapped.len() - 1) as f32 * 18.2);
    assert!(
        rich_text::add_positioned(
            &convert::blank(595., 842.).unwrap(),
            1,
            text,
            [40., 60., 150., 85.],
            &spans,
            &layout,
        )
        .is_err(),
        "extra wrapped rows must not silently exceed the block"
    );
}

#[test]
fn line_layout_remapping_handles_unicode_insertions_and_deleted_breaks() {
    let before = "İlk\nİkinci\nSon";
    let layout = vec![row(0, 0., 16.), row(5, 24., 48.), row(13, 8., 91.)];
    let same_lines = "İlk düzenleme\nİkinci\nSon";
    let mapped = rich_text::edited_layout(before, same_lines, &layout, 24.);
    assert_eq!(
        mapped
            .iter()
            .map(|line| (line.left, line.baseline))
            .collect::<Vec<_>>(),
        vec![(0., 16.), (24., 48.), (8., 91.)]
    );
    assert_eq!(
        mapped.iter().map(|line| line.start).collect::<Vec<_>>(),
        vec![
            0,
            same_lines.find("İkinci").unwrap(),
            same_lines.find("Son").unwrap()
        ]
    );
    let inserted = "İlk\nYeni\nİkinci\nSon";
    let mapped = rich_text::edited_layout(before, inserted, &layout, 24.);
    assert_eq!(
        mapped,
        vec![
            row(0, 0., 16.),
            row(5, 0., 40.),
            row(10, 24., 72.),
            row(18, 8., 115.)
        ]
    );
    let restored = rich_text::edited_layout(inserted, before, &mapped, 24.);
    assert_eq!(restored, layout);
    assert_eq!(
        rich_text::edited_layout(before, "", &layout, 24.),
        vec![layout[0]]
    );
    assert!(rich_text::edited_layout(before, inserted, &[], 24.).is_empty());
}

#[test]
fn positioned_output_rejects_stale_or_non_finite_line_origins() {
    let text = "One\nTwo";
    let spans = vec![Span {
        range: 0..text.len(),
        font: None,
        size: 14.,
        color: [0.; 3],
    }];
    let blank = convert::blank(595., 842.).unwrap();
    for layout in [
        vec![row(0, 0., 14.)],
        vec![row(0, 0., 14.), row(3, 0., 38.)],
        vec![row(0, 0., 14.), row(4, f32::NAN, 38.)],
        vec![row(0, 0., 14.), row(4, 0., 10.)],
    ] {
        assert!(
            rich_text::add_positioned(&blank, 1, text, [40., 60., 150., 80.], &spans, &layout)
                .is_err()
        );
    }
}
