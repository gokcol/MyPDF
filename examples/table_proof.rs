//! Repeatable visual proof using fictional table content.
use lopdf::{Dictionary, Document, Stream};
use mypdf_milestone0::{
    content, convert,
    tables::{self, Table},
};
fn main() -> anyhow::Result<()> {
    let root = std::path::PathBuf::from(std::env::args().nth(1).expect("output directory"));
    std::fs::create_dir_all(&root)?;
    let source = content::add_text(
        &convert::blank(595., 842.)?,
        1,
        "Quarterly review",
        [50., 55., 540., 90.],
        22.,
        [0.1, 0.15, 0.2],
    )?;
    let mut t = Table::new(4, 3, [50., 130., 545., 350.]);
    t.apply_template(1);
    t.cells = vec![
        "Item",
        "Owner",
        "Status",
        "Annual review",
        "Deniz",
        "In progress",
        "Supplier audit",
        "Orhan",
        "Approved",
        "Follow-up",
        "Selin",
        "",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    let bytes = tables::write(&source, 1, &t)?;
    std::fs::write(root.join("new-table.pdf"), &bytes)?;
    let mut doc = Document::load_mem(&bytes)?;
    let id = doc.get_pages()[&1];
    let mut ops = doc.get_and_decode_page_content(id)?;
    for op in &mut ops.operations {
        if op.operator == "BDC"
            && let Some(d) = op.operands.get_mut(1).and_then(|o| o.as_dict_mut().ok())
        {
            d.remove(b"MyPDFTable");
        }
    }
    let stream = doc.add_object(Stream::new(Dictionary::new(), ops.encode()?));
    doc.get_dictionary_mut(id)?.set("Contents", stream);
    let external = content::serialize(&mut doc)?;
    std::fs::write(root.join("existing-table.pdf"), &external)?;
    let found = tables::detect(&external, 1)?;
    anyhow::ensure!(found.len() == 1, "Expected one grid");
    let edited = tables::edit_cell(&external, 1, &found[0], 5, "İstanbul: ready")?;
    std::fs::write(root.join("edited-table.pdf"), edited)?;
    let detached = tables::detach(&external, 1, &found[0])?;
    std::fs::write(root.join("detached-table.pdf"), &detached)?;
    let doc = Document::load_mem(&detached)?;
    let mut marks = mypdf_milestone0::annotations::load(&doc);
    let mut moved = tables::from_layer(&marks[0]).unwrap();
    moved.translate(0., 120.);
    moved.apply_template(2);
    marks[0] = tables::layer(&moved, 0, Some(&marks[0]))?;
    let moved = mypdf_milestone0::annotations::export(&detached, &marks)?;
    std::fs::write(root.join("moved-layer.pdf"), moved)?;
    for which in 0..tables::TEMPLATES.len() {
        t.apply_template(which);
        std::fs::write(
            root.join(format!("template-{which}.pdf")),
            tables::write(&source, 1, &t)?,
        )?;
    }
    Ok(())
}
