use lopdf::{Document, Object, Stream, dictionary};
use mypdf_milestone0::{
    annotations::{self, Annotation, Kind, Session, Style},
    pages::{self, Operation},
};
use std::sync::Arc;

fn fixture() -> Vec<u8> {
    let mut doc = Document::with_version("1.7");
    let font =
        doc.add_object(dictionary! {"Type"=>"Font","Subtype"=>"Type1","BaseFont"=>"Helvetica"});
    let pages_id = doc.new_object_id();
    let resources = doc.add_object(dictionary! {"Font"=>dictionary!{"F1"=>font}});
    let mut ids = vec![];
    for (i, (w, h)) in [(400, 600), (600, 400), (400, 600)].into_iter().enumerate() {
        let contents=doc.add_object(Stream::new(dictionary!{},format!("0.93 0.96 0.99 rg 0 0 {w} {h} re f 0.1 0.2 0.3 rg BT /F1 32 Tf 40 {} Td (PAGE {}) Tj ET 0.2 0.5 0.8 RG 3 w 30 30 {} {} re S",h-70,i+1,w-60,h-60).into_bytes()));
        let id=doc.add_object(dictionary!{"Type"=>"Page","Parent"=>pages_id,"MediaBox"=>vec![0.into(),0.into(),w.into(),h.into()],"Contents"=>contents});
        ids.push(id);
    }
    doc.objects.insert(pages_id,dictionary!{"Type"=>"Pages","Count"=>3,"Kids"=>ids.iter().copied().map(Object::Reference).collect::<Vec<_>>(),"Resources"=>resources,"Rotate"=>90}.into());
    let outline=doc.add_object(dictionary!{"Title"=>Object::string_literal("Second page"),"Dest"=>vec![Object::Reference(ids[1]),Object::Name(b"Fit".to_vec())]});
    let outlines = doc
        .add_object(dictionary! {"Type"=>"Outlines","First"=>outline,"Last"=>outline,"Count"=>1});
    doc.get_dictionary_mut(outline)
        .unwrap()
        .set("Parent", outlines);
    let root =
        doc.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages_id,"Outlines"=>outlines});
    doc.trailer.set("Root", root);
    let mut out = vec![];
    doc.save_to(&mut out).unwrap();
    out
}
fn count(bytes: &[u8]) -> usize {
    Document::load_mem(bytes).unwrap().get_pages().len()
}
fn labels(bytes: &[u8]) -> Vec<String> {
    let doc = Document::load_mem(bytes).unwrap();
    (1..=doc.get_pages().len() as u32)
        .map(|i| doc.extract_text(&[i]).unwrap().trim().to_string())
        .collect()
}
fn annotated() -> Vec<u8> {
    annotations::export(
        &fixture(),
        &[Annotation::new(
            1,
            [60.0, 70.0, 180.0, 110.0],
            Kind::Line {
                start: [0.0, 0.5],
                end: [1.0, 0.5],
                arrow: true,
            },
            Style::default(),
        )],
    )
    .unwrap()
}

#[test]
fn reorder_preserves_inherited_resources_rotation_and_bookmark_targets() {
    let original = fixture();
    let changed = pages::apply(&original, &Operation::Reorder(vec![2, 0, 1])).unwrap();
    assert_eq!(labels(&changed), ["PAGE 3", "PAGE 1", "PAGE 2"]);
    let geometry = annotations::page_geometry(&changed).unwrap();
    assert_eq!(geometry[0].0, [0.0, 0.0, 600.0, 400.0]);
    let doc = Document::load_mem(&changed).unwrap();
    let root = doc
        .get_dictionary(
            doc.catalog()
                .unwrap()
                .get(b"Outlines")
                .unwrap()
                .as_reference()
                .unwrap(),
        )
        .unwrap();
    let entry = doc
        .get_dictionary(root.get(b"First").unwrap().as_reference().unwrap())
        .unwrap();
    assert_eq!(
        entry.get(b"Dest").unwrap().as_array().unwrap()[0]
            .as_reference()
            .unwrap(),
        doc.get_pages()[&3]
    );
    assert!(pages::apply(&original, &Operation::Reorder(vec![0, 0, 2])).is_err());
}
#[test]
fn insert_merge_extract_delete_preserve_contents_and_editable_artwork() {
    let original = annotated();
    let blank = pages::apply(
        &original,
        &Operation::InsertBlank {
            at: 1,
            width: 200.0,
            height: 300.0,
        },
    )
    .unwrap();
    assert_eq!(count(&blank), 4);
    assert_eq!(
        annotations::load(&Document::load_mem(&blank).unwrap())[0].page,
        2
    );
    let merged = pages::apply(
        &blank,
        &Operation::InsertPdf {
            at: 4,
            bytes: original.clone(),
        },
    )
    .unwrap();
    assert_eq!(count(&merged), 7);
    let a = annotations::load(&Document::load_mem(&merged).unwrap());
    assert_eq!(a.len(), 2);
    assert_ne!(a[0].id, a[1].id);
    let extracted = pages::extract(&merged, &[5, 0, 2]).unwrap();
    assert_eq!(labels(&extracted), ["PAGE 2", "PAGE 1", "PAGE 2"]);
    assert_eq!(
        annotations::load(&Document::load_mem(&extracted).unwrap()).len(),
        2
    );
    let deleted = pages::apply(&merged, &Operation::Delete(vec![0, 1, 2, 3])).unwrap();
    assert_eq!(labels(&deleted), ["PAGE 1", "PAGE 2", "PAGE 3"]);
    assert!(pages::apply(&deleted, &Operation::Delete(vec![0, 1, 2])).is_err());
    assert_eq!(count(&original), 3);
}
#[test]
fn rotation_and_crop_keep_annotation_pdf_coordinates_and_restore_full_page() {
    let original = annotated();
    let a = annotations::load(&Document::load_mem(&original).unwrap()).remove(0);
    let geometry = annotations::page_geometry(&original).unwrap();
    let pdf_point = |a: &Annotation, geometry: &Vec<(annotations::Box2, [f64; 6])>| {
        let p = a.rotated(a.local([0.0, 0.5]));
        hayro::kurbo::Affine::new(geometry[a.page].1)
            * hayro::kurbo::Point::new(p[0] as f64, p[1] as f64)
    };
    let initial = pdf_point(&a, &geometry);
    let mut changed = original.clone();
    for _ in 0..4 {
        changed = pages::apply(
            &changed,
            &Operation::Rotate {
                pages: vec![1],
                clockwise: true,
            },
        )
        .unwrap();
        let current = annotations::load(&Document::load_mem(&changed).unwrap()).remove(0);
        let point = pdf_point(&current, &annotations::page_geometry(&changed).unwrap());
        assert!((point - initial).hypot() < 0.001);
    }
    changed = pages::apply(
        &changed,
        &Operation::Crop {
            pages: vec![1],
            margins: [10.0, 20.0, 30.0, 40.0],
        },
    )
    .unwrap();
    let current = annotations::load(&Document::load_mem(&changed).unwrap()).remove(0);
    let cropped = annotations::page_geometry(&changed).unwrap();
    assert_eq!(cropped[1].0, [0.0, 0.0, 360.0, 540.0]);
    assert!((pdf_point(&current, &cropped) - initial).hypot() < 0.001);
    changed = pages::apply(&changed, &Operation::ResetCrop(vec![1])).unwrap();
    assert_eq!(
        annotations::page_geometry(&changed).unwrap()[1].0,
        geometry[1].0
    );
    assert!(
        pages::apply(
            &original,
            &Operation::Crop {
                pages: vec![0, 1],
                margins: [500.0; 4]
            }
        )
        .is_err()
    );
}
#[test]
fn combined_annotation_and_page_history_is_reversible_and_tracks_saved_revision() {
    let bytes: Arc<[u8]> = fixture().into();
    let mut session = Session::new(bytes.clone(), vec![], true);
    session
        .commit(vec![Annotation::new(
            0,
            [50.0, 60.0, 150.0, 100.0],
            Kind::Rectangle,
            Style::default(),
        )])
        .unwrap();
    let annotation_revision = session.revision();
    let result = pages::apply(
        &session.snapshot_bytes().unwrap(),
        &Operation::Reorder(vec![2, 1, 0]),
    )
    .unwrap();
    session.commit_document(result).unwrap();
    assert_eq!(session.annotations[0].page, 2);
    let page_revision = session.revision();
    session.mark_saved(page_revision);
    assert!(!session.dirty());
    session.undo();
    assert_eq!(session.revision(), annotation_revision);
    assert_eq!(session.annotations[0].page, 0);
    assert!(Arc::ptr_eq(&session.source, &bytes));
    assert!(session.dirty());
    session.undo();
    assert!(session.annotations.is_empty());
    assert_eq!(session.snapshot_bytes().unwrap(), bytes.as_ref());
    session.redo();
    session.redo();
    assert_eq!(session.annotations[0].page, 2);
    assert!(!session.dirty());
}
#[test]
fn page_tools_refuse_encryption_and_change_signed_pdfs_only_after_unlock() {
    let original = fixture();
    for kind in ["signature", "certification", "encryption"] {
        let mut doc = Document::load_mem(&original).unwrap();
        match kind {
            "signature" => {
                doc.add_object(dictionary!{"Type"=>"Sig","ByteRange"=>vec![0.into(),1.into(),2.into(),3.into()]});
            }
            "certification" => {
                doc.catalog_mut().unwrap().set("Perms", dictionary! {});
            }
            _ => {
                doc.trailer.set("Encrypt", dictionary! {});
            }
        }
        let mut bytes = vec![];
        doc.save_to(&mut bytes).unwrap();
        assert!(
            pages::apply(&bytes, &Operation::Delete(vec![0])).is_err(),
            "{kind}"
        );
        // Copying pages out never changes the source, so only encryption stops it.
        assert_eq!(
            pages::extract(&bytes, &[0]).is_ok(),
            kind != "encryption",
            "{kind}"
        );
    }
}

/// The three-page fixture with a text field on each page, named A, B and C,
/// plus a parent field "Group" with widgets on pages 1 and 3.
fn with_fields(names: [&str; 3]) -> Vec<u8> {
    let mut doc = Document::load_mem(&fixture()).unwrap();
    let pages: Vec<_> = doc.get_pages().into_values().collect();
    let mut fields = vec![];
    for (i, name) in names.iter().enumerate() {
        let field = doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx",
            "T" => Object::string_literal(*name), "V" => Object::string_literal(format!("value {name}")),
            "Rect" => vec![50.into(), 50.into(), 200.into(), 80.into()], "P" => pages[i],
        });
        doc.get_dictionary_mut(pages[i])
            .unwrap()
            .set("Annots", vec![Object::Reference(field)]);
        fields.push(Object::Reference(field));
    }
    let group = doc.new_object_id();
    let mut kids = vec![];
    for i in [0, 2] {
        let kid = doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "Parent" => group,
            "Rect" => vec![50.into(), 100.into(), 200.into(), 130.into()], "P" => pages[i],
        });
        let annots = doc.get_dictionary_mut(pages[i]).unwrap();
        let mut list = annots.get(b"Annots").unwrap().as_array().unwrap().clone();
        list.push(Object::Reference(kid));
        annots.set("Annots", list);
        kids.push(Object::Reference(kid));
    }
    doc.objects.insert(
        group,
        dictionary! { "FT" => "Tx", "T" => Object::string_literal("Group"), "V" => Object::string_literal("shared"), "Kids" => kids }.into(),
    );
    fields.push(Object::Reference(group));
    doc.catalog_mut()
        .unwrap()
        .set("AcroForm", dictionary! { "Fields" => fields });
    let mut bytes = vec![];
    doc.save_to(&mut bytes).unwrap();
    bytes
}

fn field_pages(bytes: &[u8]) -> Vec<(String, Vec<usize>)> {
    let mut fields: Vec<_> = mypdf_milestone0::forms::fields(bytes)
        .unwrap()
        .into_iter()
        .map(|f| {
            let mut pages: Vec<_> = f.widgets.iter().map(|w| w.page).collect();
            pages.sort();
            (f.name, pages)
        })
        .collect();
    fields.sort();
    fields
}

#[test]
fn pages_with_form_fields_reorder_delete_and_merge() {
    let source = with_fields(["A", "B", "C"]);
    assert_eq!(
        field_pages(&source),
        vec![
            ("A".into(), vec![0]),
            ("B".into(), vec![1]),
            ("C".into(), vec![2]),
            ("Group".into(), vec![0, 2]),
        ]
    );
    // Reordering moves each field with its page.
    let reordered = pages::apply(&source, &Operation::Reorder(vec![2, 0, 1])).unwrap();
    assert_eq!(
        field_pages(&reordered),
        vec![
            ("A".into(), vec![1]),
            ("B".into(), vec![2]),
            ("C".into(), vec![0]),
            ("Group".into(), vec![0, 1]),
        ]
    );
    // Deleting a page removes its own fields; a field with widgets elsewhere stays.
    let deleted = pages::apply(&source, &Operation::Delete(vec![0])).unwrap();
    assert_eq!(
        field_pages(&deleted),
        vec![
            ("B".into(), vec![0]),
            ("C".into(), vec![1]),
            ("Group".into(), vec![1]),
        ]
    );
    let values: Vec<_> = mypdf_milestone0::forms::fields(&deleted)
        .unwrap()
        .into_iter()
        .map(|f| f.value)
        .collect();
    assert!(values.contains(&"shared".to_owned()) && values.contains(&"value B".to_owned()));

    // Inserting another form brings its fields; a clashing name is renamed.
    let other = with_fields(["A", "X", "Y"]);
    let merged = pages::apply(
        &source,
        &Operation::InsertPdf {
            at: 3,
            bytes: other,
        },
    )
    .unwrap();
    let names: Vec<String> = field_pages(&merged).into_iter().map(|(n, _)| n).collect();
    for expected in ["A", "A (2)", "B", "C", "Group", "Group (2)", "X", "Y"] {
        assert!(
            names.contains(&expected.to_owned()),
            "{expected} in {names:?}"
        );
    }
    let a2 = mypdf_milestone0::forms::fields(&merged)
        .unwrap()
        .into_iter()
        .find(|f| f.name == "A (2)")
        .unwrap();
    assert_eq!(a2.value, "value A");
    assert_eq!(a2.widgets[0].page, 3);

    // Extracting keeps the fields on the extracted pages only.
    let extracted = pages::extract(&source, &[1, 2]).unwrap();
    assert_eq!(
        field_pages(&extracted),
        vec![
            ("B".into(), vec![0]),
            ("C".into(), vec![1]),
            ("Group".into(), vec![1]),
        ]
    );
}

#[test]
fn ranges_are_validated_and_safe_export_never_overwrites() {
    assert_eq!(
        pages::parse_range("1, 3-5, 3", 5).unwrap(),
        vec![0, 2, 3, 4]
    );
    for bad in ["", "0", "2-1", "1-6", "1--2"] {
        assert!(pages::parse_range(bad, 5).is_err());
    }
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("original.pdf");
    let source = fixture();
    mypdf_milestone0::write_new(&path, &source).unwrap();
    assert!(mypdf_milestone0::write_new(&path, &pages::extract(&source, &[0]).unwrap()).is_err());
    assert_eq!(std::fs::read(path).unwrap(), source);
}
#[test]
#[ignore = "Creates visual proof files when MYPDF_PAGE_PROOF_DIR is set"]
fn page_operations_visual_proof() {
    let root = std::path::PathBuf::from(
        std::env::var_os("MYPDF_PAGE_PROOF_DIR").expect("proof output directory"),
    );
    std::fs::create_dir_all(&root).unwrap();
    let source = annotated();
    std::fs::write(root.join("source.pdf"), &source).unwrap();
    let mut current = pages::apply(&source, &Operation::Reorder(vec![2, 0, 1])).unwrap();
    current = pages::apply(
        &current,
        &Operation::InsertBlank {
            at: 1,
            width: 400.0,
            height: 600.0,
        },
    )
    .unwrap();
    current = pages::apply(
        &current,
        &Operation::InsertPdf {
            at: 4,
            bytes: source.clone(),
        },
    )
    .unwrap();
    current = pages::apply(&current, &Operation::Delete(vec![1])).unwrap();
    current = pages::apply(
        &current,
        &Operation::Rotate {
            pages: vec![0, 2],
            clockwise: true,
        },
    )
    .unwrap();
    current = pages::apply(
        &current,
        &Operation::Crop {
            pages: vec![2],
            margins: [20.0, 30.0, 40.0, 50.0],
        },
    )
    .unwrap();
    std::fs::write(root.join("organized.pdf"), &current).unwrap();
    std::fs::write(
        root.join("extracted.pdf"),
        pages::extract(&current, &[0, 2, 5]).unwrap(),
    )
    .unwrap();
    for (i, chunk) in (0..6).collect::<Vec<_>>().chunks(2).enumerate() {
        std::fs::write(
            root.join(format!("split-{}.pdf", i + 1)),
            pages::extract(&current, chunk).unwrap(),
        )
        .unwrap();
    }
}
