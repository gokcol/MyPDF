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
