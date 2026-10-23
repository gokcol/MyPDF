use lopdf::{Dictionary, Document, Stream};
use mypdf_milestone0::{
    content, convert,
    tables::{self, Table},
    text_geometry,
};
fn external_grid(bytes: &[u8]) -> Vec<u8> {
    let mut doc = Document::load_mem(bytes).unwrap();
    let id = doc.get_pages()[&1];
    let mut content = doc.get_and_decode_page_content(id).unwrap();
    for op in &mut content.operations {
        if op.operator == "BDC"
            && let Some(d) = op.operands.get_mut(1).and_then(|o| o.as_dict_mut().ok())
        {
            d.remove(b"MyPDFTable");
        }
    }
    let stream = doc.add_object(Stream::new(Dictionary::new(), content.encode().unwrap()));
    doc.get_dictionary_mut(id).unwrap().set("Contents", stream);
    content::serialize(&mut doc).unwrap()
}
#[test]
fn tables_round_trip_unicode_edit_cells_and_keep_neighboring_content() {
    let source = content::add_text(
        &convert::blank(500., 600.).unwrap(),
        1,
        "Outside table",
        [30., 30., 300., 60.],
        12.,
        [0.; 3],
    )
    .unwrap();
    let mut t = Table::new(3, 3, [40., 100., 460., 280.]);
    t.cells[0] = "İstanbul".into();
    t.cells[4] = "Quarterly report".into();
    t.header = true;
    let out = tables::write(&source, 1, &t).unwrap();
    assert_eq!(tables::owned(&out, 1).unwrap(), vec![t.clone()]);
    let edited = tables::edit_cell(&out, 1, &t, 4, "Approved").unwrap();
    let owned = tables::owned(&edited, 1).unwrap();
    assert_eq!(owned[0].cells[4], "Approved");
    let words = text_geometry::runs(&edited, 1)
        .unwrap()
        .into_iter()
        .map(|r| r.text)
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        words.contains("Outside table") && words.contains("İstanbul") && words.contains("Approved")
    );
    assert!(!words.contains("Quarterly report"));
    assert!(
        tables::detect(&edited, 1).unwrap().is_empty(),
        "owned tables are not offered twice"
    );
}
#[test]
fn bordered_table_detection_is_non_mutating_and_cell_changes_preserve_grid() {
    let mut t = Table::new(3, 2, [40., 70., 460., 250.]);
    t.cells[0] = "Name".into();
    t.cells[1] = "Result".into();
    t.cells[2] = "Deniz".into();
    let source =
        external_grid(&tables::write(&convert::blank(500., 500.).unwrap(), 1, &t).unwrap());
    let before = source.clone();
    let found = tables::detect(&source, 1).unwrap();
    assert_eq!(source, before);
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].rows(), found[0].columns()), (3, 2));
    assert_eq!(found[0].cells[0], "Name");
    let edited = tables::edit_cell(&source, 1, &found[0], 2, "Orhan Gökçöl").unwrap();
    let found = tables::detect(&edited, 1).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].cells[2], "Orhan Gökçöl");
    assert_eq!(found[0].cells[0], "Name");
    let edited = tables::edit_cell(&edited, 1, &found[0], 5, "New entry").unwrap();
    assert_eq!(tables::detect(&edited, 1).unwrap()[0].cells[5], "New entry");
}
#[test]
fn table_geometry_obeys_page_rotation_and_rejects_overflow_without_output() {
    for rotation in [0, 90, 180, 270] {
        let source = convert::blank(500., 600.).unwrap();
        let mut doc = Document::load_mem(&source).unwrap();
        let id = doc.get_pages()[&1];
        doc.get_dictionary_mut(id).unwrap().set("Rotate", rotation);
        doc.get_dictionary_mut(id).unwrap().set(
            "CropBox",
            vec![20.into(), 30.into(), 480.into(), 570.into()],
        );
        let source = content::serialize(&mut doc).unwrap();
        let mut t = Table::new(2, 2, [40., 80., 400., 220.]);
        t.cells[0] = "Rotated table".into();
        let out = tables::write(&source, 1, &t).unwrap();
        let external = external_grid(&out);
        let detected = tables::detect(&external, 1).unwrap();
        assert_eq!(detected.len(), 1, "rotation {rotation}");
        for (a, b) in detected[0].rect().iter().zip(t.rect()) {
            assert!((a - b).abs() < 0.1, "rotation {rotation}: {a} {b}");
        }
        t.cells[0] = "This text is much too long to fit inside one short table cell. ".repeat(200);
        assert!(tables::write(&source, 1, &t).is_err());
    }
}
#[test]
fn row_and_column_changes_keep_existing_cell_values() {
    let mut t = Table::new(2, 2, [20., 20., 220., 100.]);
    t.cells = vec!["A".into(), "B".into(), "C".into(), "D".into()];
    t.resize_structure(true, 1, true).unwrap();
    assert_eq!(t.cells, vec!["A", "B", "", "", "C", "D"]);
    t.resize_structure(false, 0, true).unwrap();
    assert_eq!(t.cells, vec!["", "A", "B", "", "", "", "", "C", "D"]);
    t.resize_structure(false, 0, false).unwrap();
    t.resize_structure(true, 1, false).unwrap();
    assert_eq!(t.cells, vec!["A", "B", "C", "D"]);
    t.validate().unwrap();
}

#[test]
fn external_cell_edits_never_restore_stale_table_words() {
    let mut t = Table::new(2, 2, [30., 60., 430., 180.]);
    t.cells[0] = "Original cell".into();
    let source = tables::write(&convert::blank(500., 500.).unwrap(), 1, &t).unwrap();
    let changed =
        mypdf_milestone0::document::replace_run(&source, 1, "Original cell", "Changed elsewhere")
            .unwrap();
    assert!(tables::owned(&changed, 1).unwrap().is_empty());
    let detected = tables::detect(&changed, 1).unwrap();
    assert_eq!(detected[0].cells[0], "Changed elsewhere");
}

#[test]
fn templates_and_scoped_styles_survive_writing_and_structure_changes() {
    use tables::{CellStyle, Scope};
    let source = convert::blank(500., 600.).unwrap();
    for which in 0..tables::TEMPLATES.len() {
        let mut t = Table::new(3, 3, [40., 80., 460., 300.]);
        t.cells[0] = "İstanbul".into();
        t.apply_template(which);
        let original = t.style(8);
        let custom = CellStyle {
            color: [0.7, 0.1, 0.2],
            insets: [7., 3., 5., 2.],
            bold: true,
            italic: true,
            alignment: 2,
            vertical: 2,
            ..Default::default()
        };
        t.set_style(4, Scope::Column, custom.clone());
        assert_eq!(t.style(1), custom);
        assert_eq!(t.style(7), custom);
        assert_eq!(t.style(8), original);
        let out = tables::write(&source, 1, &t).unwrap();
        assert_eq!(tables::owned(&out, 1).unwrap()[0], t);
        t.resize_structure(true, 1, true).unwrap();
        assert_eq!(t.style(10), custom);
        t.resize_structure(true, 1, false).unwrap();
        assert_eq!(t.style(7), custom);
    }
}

#[test]
fn table_layers_are_vector_editable_after_reload_and_follow_the_annotation_rect() {
    use mypdf_milestone0::annotations;
    let mut t = Table::new(2, 2, [40., 80., 440., 240.]);
    t.cells[0] = "İstanbul & <Review>".into();
    t.apply_template(2);
    let mark = tables::layer(&t, 0, None).unwrap();
    let source = convert::blank(500., 600.).unwrap();
    let bytes = annotations::export(&source, &[mark]).unwrap();
    let doc = Document::load_mem(&bytes).unwrap();
    let mut mark = annotations::load(&doc).remove(0);
    assert_eq!(tables::from_layer(&mark).unwrap(), t);
    assert!(
        doc.objects
            .values()
            .filter_map(|o| o.as_stream().ok())
            .any(|s| s.dict.get(b"Subtype").ok().and_then(|o| o.as_name().ok()) == Some(b"Form"))
    );
    assert!(
        !doc.objects
            .values()
            .filter_map(|o| o.as_stream().ok())
            .any(|s| s.dict.get(b"Subtype").ok().and_then(|o| o.as_name().ok()) == Some(b"Image")),
        "The table must remain vector artwork"
    );
    mark.rect = [60., 110., 460., 310.];
    let moved = tables::from_layer(&mark).unwrap();
    assert_eq!(moved.rect(), mark.rect);
    assert_eq!(moved.cells, t.cells);
    assert_eq!(moved.styles, t.styles);
    let updated = tables::layer(&moved, 0, Some(&mark)).unwrap();
    assert_eq!(updated.id, mark.id);
}

#[test]
fn detaching_owned_and_detected_tables_preserves_neighbors_and_is_one_undo_step() {
    use mypdf_milestone0::annotations::{self, Session};
    for rotation in [0, 90, 180, 270] {
        let mut doc = Document::load_mem(&convert::blank(500., 600.).unwrap()).unwrap();
        let page = doc.get_pages()[&1];
        doc.get_dictionary_mut(page)
            .unwrap()
            .set("Rotate", rotation);
        doc.get_dictionary_mut(page).unwrap().set(
            "CropBox",
            vec![20.into(), 30.into(), 480.into(), 570.into()],
        );
        let bytes = content::serialize(&mut doc).unwrap();
        let source = content::add_text(
            &bytes,
            1,
            "Keep outside",
            [30., 20., 400., 60.],
            12.,
            [0.; 3],
        )
        .unwrap();
        let mut t = Table::new(3, 2, [40., 90., 420., 270.]);
        t.cells[0] = "Cell words".into();
        t.cells[3] = "İstanbul".into();
        let embedded = tables::write(&source, 1, &t).unwrap();
        for foreign in [false, true] {
            let input = if foreign {
                external_grid(&embedded)
            } else {
                embedded.clone()
            };
            let table = if foreign {
                tables::detect(&input, 1).unwrap().remove(0)
            } else {
                t.clone()
            };
            let detached = tables::detach(&input, 1, &table).unwrap();
            let doc = Document::load_mem(&detached).unwrap();
            let marks = annotations::load(&doc);
            assert_eq!(marks.len(), 1, "rotation {rotation}, foreign {foreign}");
            assert_eq!(tables::from_layer(&marks[0]).unwrap().cells, table.cells);
            assert!(tables::owned(&detached, 1).unwrap().is_empty());
            assert!(
                tables::detect(&detached, 1).unwrap().is_empty(),
                "Old borders must be removed"
            );
            let words = text_geometry::runs(&detached, 1)
                .unwrap()
                .into_iter()
                .map(|r| r.text)
                .collect::<Vec<_>>()
                .join(" ");
            assert!(words.contains("Keep outside"));
            assert!(
                !words.contains("Cell words"),
                "The original embedded text must be removed"
            );
            let mut session = Session::new(input.clone().into(), vec![], true);
            session.commit_document(detached).unwrap();
            assert_eq!(session.annotations.len(), 1);
            session.undo();
            assert_eq!(session.source.as_ref(), input.as_slice());
            assert!(session.annotations.is_empty());
            session.redo();
            assert_eq!(session.annotations.len(), 1);
        }
    }
}

#[test]
fn detected_cell_formatting_persists_without_changing_neighbors() {
    use tables::Scope;
    let mut t = Table::new(2, 2, [30., 80., 450., 240.]);
    t.cells = vec![
        "Name".into(),
        "Status".into(),
        "Deniz".into(),
        "Ready".into(),
    ];
    let input = external_grid(&tables::write(&convert::blank(500., 500.).unwrap(), 1, &t).unwrap());
    let base = tables::detect(&input, 1).unwrap().remove(0);
    let mut changed = base.clone();
    let mut style = changed.style(2);
    style.bold = true;
    style.color = [0.1, 0.2, 0.7];
    style.fill = Some([0.9, 0.95, 1.]);
    style.insets = [8., 5., 8., 5.];
    style.alignment = 1;
    style.vertical = 1;
    changed.set_style(2, Scope::Row, style.clone());
    changed.cells[2] = "İstanbul".into();
    let bytes = tables::edit_detected(&input, 1, &changed, &base).unwrap();
    let found = tables::detect(&bytes, 1).unwrap().remove(0);
    assert_eq!(found.cells, changed.cells);
    assert_eq!(found.style(2), style);
    assert_eq!(found.style(3), style);
    assert_eq!(found.style(0), base.style(0));
    let mut cleared = found.clone();
    style.fill = None;
    cleared.set_style(2, Scope::Row, style.clone());
    let bytes = tables::edit_detected(&bytes, 1, &cleared, &found).unwrap();
    assert_eq!(tables::detect(&bytes, 1).unwrap()[0].style(2), style);
}

#[test]
fn table_removal_refuses_shared_border_paths_without_altering_the_pdf() {
    use lopdf::content::Operation;
    let t = Table::new(2, 2, [40., 80., 440., 200.]);
    let input = external_grid(&tables::write(&convert::blank(500., 500.).unwrap(), 1, &t).unwrap());
    let found = tables::detect(&input, 1).unwrap().remove(0);
    let mut doc = Document::load_mem(&input).unwrap();
    let page = doc.get_pages()[&1];
    let mut ops = doc.get_and_decode_page_content(page).unwrap();
    let i = ops
        .operations
        .iter()
        .position(|o| o.operator == "S")
        .unwrap();
    ops.operations.splice(
        i..i,
        [
            Operation::new("m", vec![480.into(), 480.into()]),
            Operation::new("l", vec![490.into(), 480.into()]),
        ],
    );
    let stream = doc.add_object(Stream::new(Dictionary::new(), ops.encode().unwrap()));
    doc.get_dictionary_mut(page)
        .unwrap()
        .set("Contents", stream);
    let input = content::serialize(&mut doc).unwrap();
    let before = input.clone();
    for remove in [tables::detach, tables::delete] {
        let error = remove(&input, 1, &found).unwrap_err();
        assert!(
            error.to_string().contains("share a drawing operation"),
            "{error:#}"
        );
        assert_eq!(input, before);
    }
}

#[test]
fn deleting_owned_and_detected_tables_preserves_neighbors_and_is_one_undo_step() {
    use mypdf_milestone0::annotations::{self, Annotation, Kind, Session};
    for rotation in [0, 90, 180, 270] {
        let mut doc = Document::load_mem(&convert::blank(500., 600.).unwrap()).unwrap();
        let page = doc.get_pages()[&1];
        doc.get_dictionary_mut(page)
            .unwrap()
            .set("Rotate", rotation);
        doc.get_dictionary_mut(page).unwrap().set(
            "CropBox",
            vec![20.into(), 30.into(), 480.into(), 570.into()],
        );
        let source = content::add_text(
            &content::serialize(&mut doc).unwrap(),
            1,
            "Keep outside",
            [30., 20., 400., 60.],
            12.,
            [0.; 3],
        )
        .unwrap();
        let neighbor = Annotation::new(
            0,
            [40., 320., 200., 370.],
            Kind::Rectangle,
            Default::default(),
        );
        let source = annotations::export(&source, std::slice::from_ref(&neighbor)).unwrap();
        let mut table = Table::new(3, 2, [40., 90., 420., 270.]);
        table.apply_template(1);
        table.cells[0] = "Delete these words".into();
        table.cells[3] = "İstanbul".into();
        let embedded = tables::write(&source, 1, &table).unwrap();
        for foreign in [false, true] {
            let input = if foreign {
                external_grid(&embedded)
            } else {
                embedded.clone()
            };
            let target = if foreign {
                tables::detect(&input, 1).unwrap().remove(0)
            } else {
                table.clone()
            };
            let deleted = tables::delete(&input, 1, &target).unwrap();
            assert!(tables::owned(&deleted, 1).unwrap().is_empty());
            assert!(
                tables::detect(&deleted, 1).unwrap().is_empty(),
                "rotation {rotation}, foreign {foreign}: old borders remain"
            );
            let marks = annotations::load(&Document::load_mem(&deleted).unwrap());
            assert_eq!(
                marks,
                std::slice::from_ref(&neighbor),
                "Keep the neighboring annotation without creating a table layer"
            );
            let words = text_geometry::runs(&deleted, 1)
                .unwrap()
                .into_iter()
                .map(|r| r.text)
                .collect::<Vec<_>>()
                .join(" ");
            assert!(words.contains("Keep outside"));
            assert!(!words.contains("Delete these words") && !words.contains("İstanbul"));
            let mut session = Session::new(input.clone().into(), vec![neighbor.clone()], true);
            session.commit_document(deleted).unwrap();
            assert!(tables::owned(&session.source, 1).unwrap().is_empty());
            session.undo();
            assert_eq!(session.source.as_ref(), input.as_slice());
            assert_eq!(session.annotations, vec![neighbor.clone()]);
            session.redo();
            assert!(tables::detect(&session.source, 1).unwrap().is_empty());
            assert_eq!(session.annotations, vec![neighbor.clone()]);
        }
    }
}

#[test]
fn deleting_a_table_never_requires_rendering_its_pending_cell_text() {
    let mut table = Table::new(2, 2, [40., 80., 440., 200.]);
    table.cells[0] = "Original cell".into();
    let input = tables::write(&convert::blank(500., 500.).unwrap(), 1, &table).unwrap();
    for foreign in [false, true] {
        let bytes = if foreign {
            external_grid(&input)
        } else {
            input.clone()
        };
        let mut pending = if foreign {
            tables::detect(&bytes, 1).unwrap().remove(0)
        } else {
            table.clone()
        };
        pending.cells[0] = "Text that does not fit inside the table cell. ".repeat(300);
        pending.validate().unwrap();
        assert!(tables::layer(&pending, 0, None).is_err());
        let deleted = tables::delete(&bytes, 1, &pending).unwrap();
        assert!(tables::detect(&deleted, 1).unwrap().is_empty());
        assert!(
            text_geometry::runs(&deleted, 1)
                .unwrap()
                .iter()
                .all(|r| r.text.trim().is_empty())
        );
        assert!(
            mypdf_milestone0::annotations::load(&Document::load_mem(&deleted).unwrap()).is_empty()
        );
    }
}

#[test]
fn merged_rectangles_preserve_text_and_split_without_losing_combined_words() {
    let mut table = Table::new(3, 3, [20., 30., 320., 210.]);
    table.cells = (0..9).map(|i| format!("Cell {i}")).collect();
    table.cells[2].clear();
    let neighbors = [
        table.cells[0].clone(),
        table.cells[3].clone(),
        table.cells[8].clone(),
    ];
    assert_eq!(table.merge_range(1, 5).unwrap(), 1);
    assert_eq!(table.span(1), [2, 2]);
    assert_eq!(table.cell(1), [120., 30., 320., 150.]);
    assert_eq!(table.cell(5), table.cell(1));
    for covered in [2, 4, 5] {
        assert_eq!(table.anchor(covered), 1);
        assert!(table.is_covered(covered));
        assert!(table.cells[covered].is_empty());
    }
    assert!(!table.is_covered(1));
    assert_eq!(table.cells[1], "Cell 1\nCell 4\nCell 5");
    assert_eq!(table.anchors().collect::<Vec<_>>(), vec![0, 1, 3, 6, 7, 8]);
    assert_eq!(table.hit([250., 120.]), Some(1));
    table.validate().unwrap();
    assert!(table.split_cell(5).unwrap());
    assert!(table.spans.is_empty());
    assert_eq!(table.cells[1], "Cell 1\nCell 4\nCell 5");
    assert_eq!(
        [
            table.cells[0].clone(),
            table.cells[3].clone(),
            table.cells[8].clone()
        ],
        neighbors
    );
    assert_eq!(table.anchors().count(), 9);
    assert!(!table.split_cell(1).unwrap());
    table.validate().unwrap();
}

#[test]
fn invalid_or_overlapping_merges_and_structure_changes_are_atomic() {
    let mut table = Table::new(3, 3, [20., 20., 320., 200.]);
    table.cells = (0..9).map(|i| format!("Value {i}")).collect();
    table.merge_range(0, 4).unwrap();
    let before = table.clone();
    assert!(table.merge_range(4, 8).is_err());
    assert_eq!(table, before);
    assert!(table.merge_range(2, 9).is_err());
    assert_eq!(table, before);
    for (row, at, insert) in [
        (true, 1, true),
        (true, 0, false),
        (false, 1, true),
        (false, 0, false),
    ] {
        assert!(table.resize_structure(row, at, insert).is_err());
        assert_eq!(
            table, before,
            "Split merged cells before changing structure"
        );
    }
    assert!(table.split_cell(99).is_err());
    assert_eq!(table, before);

    let mut too_long = Table::new(1, 2, [20., 20., 320., 80.]);
    too_long.cells = vec!["x".repeat(20000), "y".repeat(20000)];
    too_long.validate().unwrap();
    let before = too_long.clone();
    assert!(too_long.merge_range(0, 1).is_err());
    assert_eq!(
        too_long, before,
        "Combining cells must keep the cell text limit"
    );
}

#[test]
fn malformed_span_metadata_is_rejected_without_hiding_cell_text() {
    use tables::CellSpan;
    let mut table = Table::new(3, 3, [20., 20., 320., 200.]);
    for spans in [
        vec![CellSpan {
            anchor: 8,
            rows: 2,
            columns: 1,
        }],
        vec![CellSpan {
            anchor: 2,
            rows: 1,
            columns: 2,
        }],
        vec![CellSpan {
            anchor: 0,
            rows: 0,
            columns: 2,
        }],
        vec![CellSpan {
            anchor: usize::MAX,
            rows: 1,
            columns: 2,
        }],
        vec![CellSpan {
            anchor: 0,
            rows: usize::MAX,
            columns: 1,
        }],
        vec![
            CellSpan {
                anchor: 0,
                rows: 2,
                columns: 2,
            },
            CellSpan {
                anchor: 4,
                rows: 2,
                columns: 2,
            },
        ],
    ] {
        table.spans = spans;
        assert!(table.validate().is_err());
    }
    table.spans = vec![CellSpan {
        anchor: 0,
        rows: 1,
        columns: 2,
    }];
    table.cells[1] = "Do not hide these words".into();
    assert!(table.validate().is_err());
}

#[test]
fn merged_tables_round_trip_as_owned_content_and_movable_vector_layers() {
    use mypdf_milestone0::annotations;
    let source = convert::blank(500., 600.).unwrap();
    let mut table = Table::new(3, 3, [40., 80., 460., 380.]);
    table.apply_template(1);
    table.cells[0] = "Orhan Gökçöl".into();
    table.cells[1] = "İstanbul".into();
    table.cells[8] = "Keep this neighbor".into();
    table.merge_range(0, 4).unwrap();
    table.auto_height = true;
    let embedded = tables::write(&source, 1, &table).unwrap();
    assert_eq!(tables::owned(&embedded, 1).unwrap(), vec![table.clone()]);
    let words = text_geometry::runs(&embedded, 1)
        .unwrap()
        .into_iter()
        .map(|r| r.text)
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        words.contains("Orhan Gökçöl")
            && words.contains("İstanbul")
            && words.contains("Keep this neighbor")
    );
    let mark = tables::layer(&table, 0, None).unwrap();
    let bytes = annotations::export(&source, &[mark]).unwrap();
    let document = Document::load_mem(&bytes).unwrap();
    let mut marks = annotations::load(&document);
    assert_eq!(marks.len(), 1);
    assert_eq!(tables::from_layer(&marks[0]).unwrap(), table);
    assert!(
        !document
            .objects
            .values()
            .filter_map(|o| o.as_stream().ok())
            .any(|s| s.dict.get(b"Subtype").ok().and_then(|o| o.as_name().ok()) == Some(b"Image")),
        "Merged tables must stay vector artwork"
    );
    marks[0].rect = [60., 100., 480., 400.];
    let moved = tables::from_layer(&marks[0]).unwrap();
    assert_eq!(moved.rect(), marks[0].rect);
    assert_eq!(moved.spans, table.spans);
    assert_eq!(moved.cells, table.cells);
    assert!(moved.auto_height);
}

#[test]
fn merging_all_cells_removes_internal_borders_in_pdf_and_svg() {
    use mypdf_milestone0::annotations::Kind;
    let mut table = Table::new(2, 2, [40., 80., 440., 240.]);
    table.merge_range(0, 3).unwrap();
    let bytes = tables::write(&convert::blank(500., 600.).unwrap(), 1, &table).unwrap();
    let document = Document::load_mem(&bytes).unwrap();
    let page = document.get_pages()[&1];
    let content = document.get_and_decode_page_content(page).unwrap();
    let rectangles = content
        .operations
        .iter()
        .filter(|o| o.operator == "re")
        .collect::<Vec<_>>();
    assert_eq!(
        rectangles.len(),
        1,
        "Only the merged outside border may be drawn"
    );
    let coordinates = rectangles[0]
        .operands
        .iter()
        .map(|o| o.as_float().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(coordinates, vec![40., 80., 400., 160.]);
    assert!(
        !content.operations.iter().any(|o| o.operator == "l"),
        "No internal border line may cross the merged cell"
    );
    let mark = tables::layer(&table, 0, None).unwrap();
    let Kind::Svg { source, .. } = mark.kind else {
        panic!("Expected a vector table layer")
    };
    assert_eq!(source.matches("<rect ").count(), 1);
    assert!(!source.contains("<line") && !source.contains("<path"));
}

#[test]
fn legacy_table_metadata_defaults_to_unmerged_fixed_height() {
    let table = Table::new(2, 2, [20., 20., 220., 100.]);
    let mut old = serde_json::to_value(&table).unwrap();
    let fields = old.as_object_mut().unwrap();
    fields.remove("spans");
    fields.remove("auto_height");
    let restored: Table = serde_json::from_value(old).unwrap();
    assert!(restored.spans.is_empty());
    assert!(!restored.auto_height);
    assert_eq!(restored, table);
}

#[test]
fn spreadsheet_paste_handles_quoted_unicode_multiline_and_empty_fields() {
    let mut table = Table::new(3, 3, [20., 20., 320., 200.]);
    table.cells.fill("Before".into());
    let clipboard = "\"İstanbul\tNorth\"\t\"Line 1\nLine 2\"\r\n\"Say \"\"yes\"\"\"\t\r\n";
    assert_eq!(table.paste_grid(0, clipboard, false).unwrap(), 4);
    assert_eq!(table.cells[0], "İstanbul\tNorth");
    assert_eq!(table.cells[1], "Line 1\nLine 2");
    assert_eq!(table.cells[3], "Say \"yes\"");
    assert_eq!(table.cells[4], "");
    assert_eq!(
        table.cells[6], "Before",
        "Trailing CRLF is a terminator, not a blank row"
    );
    let before = table.clone();
    assert!(table.paste_grid(6, "A\tB\tC\nD", false).is_err());
    assert_eq!(table, before);
    assert_eq!(table.paste_grid(0, "A\tB\nC", false).unwrap(), 4);
    assert_eq!(
        (
            table.cells[0].as_str(),
            table.cells[1].as_str(),
            table.cells[3].as_str(),
            table.cells[4].as_str()
        ),
        ("A", "B", "C", "")
    );
    assert_eq!(table.paste_grid(0, "", false).unwrap(), 1);
    assert!(table.cells[0].is_empty());
}

#[test]
fn spreadsheet_paste_growth_preserves_cells_styles_and_remaps_existing_merges() {
    let mut table = Table::new(3, 2, [20., 20., 220., 140.]);
    table.cells = vec![
        "A".into(),
        "B".into(),
        "C".into(),
        "D".into(),
        "E".into(),
        "F".into(),
    ];
    table.apply_template(2);
    table.styles[3].color = [0.7, 0.2, 0.1];
    let covered_style = table.style(3);
    let a_style = table.style(0);
    let e_style = table.style(4);
    table.merge_range(2, 3).unwrap();
    assert_eq!(table.paste_grid(5, "G\tH\nİ\tJ", true).unwrap(), 4);
    assert_eq!((table.rows(), table.columns()), (4, 3));
    assert_eq!(
        table.cells,
        vec!["A", "B", "", "C\nD", "", "", "E", "G", "H", "", "İ", "J"]
    );
    assert_eq!(table.anchor(4), 3);
    assert_eq!(table.span(3), [1, 2]);
    assert_eq!(table.style(0), a_style);
    assert_eq!(table.style(6), e_style);
    table.validate().unwrap();
    table.split_cell(4).unwrap();
    assert_eq!(
        table.style(4),
        covered_style,
        "Growth must preserve properties of cells uncovered by splitting later"
    );
}

#[test]
fn invalid_spreadsheet_paste_never_changes_cells_geometry_styles_or_merges() {
    let mut table = Table::new(3, 3, [20., 20., 320., 200.]);
    table.cells[0] = "Keep anchor".into();
    table.cells[8] = "Keep neighbor".into();
    table.apply_template(1);
    table.merge_range(0, 1).unwrap();
    let before = table.clone();
    for (at, text, grow) in [
        (0, "Replace anchor", false),
        (1, "Replace covered", false),
        (2, "A\tB", false),
        (8, "A\nB", false),
        (9, "Bad start", true),
        (4, "\"Unclosed\tfield", true),
    ] {
        assert!(table.paste_grid(at, text, grow).is_err());
        assert_eq!(
            table, before,
            "Rejected paste at {at} must be transactional"
        );
    }
}

#[test]
fn spreadsheet_paste_enforces_dimension_cell_and_total_text_limits_atomically() {
    let mut table = Table::new(1, 1, [20., 20., 220., 60.]);
    for clipboard in [
        vec!["x"; 61].join("\n"),
        vec!["x"; 13].join("\t"),
        "é".repeat(16385),
    ] {
        let before = table.clone();
        assert!(table.paste_grid(0, &clipboard, true).is_err());
        assert_eq!(table, before);
    }
    let mut table = Table::new(18, 1, [20., 20., 320., 560.]);
    for cell in &mut table.cells[..16] {
        *cell = "x".repeat(32000);
    }
    table.validate().unwrap();
    let before = table.clone();
    assert!(table.paste_grid(16, &"y".repeat(32000), false).is_err());
    assert_eq!(table, before);
    let mut full = Table::new(60, 12, [20., 20., 260., 1220.]);
    assert_eq!(
        full.paste_grid(0, &vec![["x"; 12].join("\t"); 60].join("\n"), false)
            .unwrap(),
        720
    );
    let before = full.clone();
    assert!(full.paste_grid(719, "x\ty", true).is_err());
    assert_eq!(full, before);
}

#[test]
fn fitted_row_heights_use_wrapping_metrics_and_shrink_when_text_is_shorter() {
    let mut table = Table::new(2, 2, [20., 20., 260., 60.]);
    table.auto_height = true;
    table.cells[0] = "Alpha beta gamma delta epsilon zeta eta theta iota kappa lambda.".into();
    assert!(tables::layer(&table, 0, None).is_err());
    table.fit_row_heights(500.).unwrap();
    assert!(table.ys[1] - table.ys[0] > 20.);
    let grown = table.ys.clone();
    tables::layer(&table, 0, None).unwrap();
    tables::write(&convert::blank(500., 600.).unwrap(), 1, &table).unwrap();
    table.cells[0] = "Short".into();
    table.fit_row_heights(500.).unwrap();
    assert!(table.ys[1] - table.ys[0] < grown[1] - grown[0]);
    assert!(table.ys.windows(2).all(|p| p[1] - p[0] >= 12.));
    tables::layer(&table, 0, None).unwrap();
}

#[test]
fn row_height_fit_failure_is_atomic_even_with_merged_multiline_cells() {
    let mut table = Table::new(3, 2, [20., 20., 260., 80.]);
    table.auto_height = true;
    table.cells[0] = "Many lines\nare needed\nfor this\nmerged cell\nwith content".into();
    table.merge_range(0, 3).unwrap();
    table.apply_template(1);
    let before = table.clone();
    assert!(table.fit_row_heights(50.).is_err());
    assert_eq!(
        table, before,
        "A failed fit must retain all original row boundaries"
    );
    table.fit_row_heights(500.).unwrap();
    table.validate().unwrap();
    tables::layer(&table, 0, None).unwrap();
}

#[test]
fn bordered_table_detection_preserves_horizontal_and_vertical_merged_cells() {
    let mut table = Table::new(3, 3, [40., 80., 460., 380.]);
    table.cells = vec![
        "İstanbul".into(),
        "North".into(),
        "Vertical top".into(),
        "Middle left".into(),
        "Middle center".into(),
        "Vertical bottom".into(),
        "Bottom left".into(),
        "Bottom center".into(),
        "Bottom right".into(),
    ];
    table.merge_range(0, 1).unwrap();
    table.merge_range(2, 5).unwrap();
    let source =
        external_grid(&tables::write(&convert::blank(500., 600.).unwrap(), 1, &table).unwrap());
    let before = source.clone();
    let detected = tables::detect(&source, 1).unwrap();
    assert_eq!(source, before);
    assert_eq!(detected.len(), 1);
    let base = &detected[0];
    assert_eq!((base.rows(), base.columns()), (3, 3));
    assert_eq!(base.spans, table.spans);
    assert_eq!(base.cells, table.cells);
    for (actual, expected) in base.rect().iter().zip(table.rect()) {
        assert!((actual - expected).abs() < 0.1);
    }
    let mut split = base.clone();
    split.split_cell(0).unwrap();
    split.cells[8] = "Must not partially apply".into();
    assert!(tables::edit_detected(&source, 1, &split, base).is_err());
    let mut merged = base.clone();
    merged.merge_range(6, 7).unwrap();
    assert!(tables::edit_detected(&source, 1, &merged, base).is_err());
    assert_eq!(source, before);
    assert_eq!(tables::detect(&source, 1).unwrap()[0], *base);
}
