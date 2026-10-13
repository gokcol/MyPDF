//! Diagnose Acrobat's stricter PDF-structure checks with controlled save variants.
use anyhow::Result;
use lopdf::{Dictionary, Document, Object, Stream, dictionary};
use mypdf_milestone0::{identity, signing, write_new};
use std::path::PathBuf;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let out = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "artifacts/interop-probe".into()),
    );
    let signer = identity::FileSigner::load(
        &std::fs::read("artifacts/milestone0-final/test-rsa.p12")?,
        "MyPDF-milestone0-synthetic-only",
        None,
    )?;
    let edited = std::fs::read("artifacts/milestone0-final/edited.pdf")?;
    for (name, table, dense) in [
        ("table-dense", true, true),
        ("stream-dense", false, true),
        ("table-sparse", true, false),
    ] {
        let mut doc = Document::load_mem(&edited)?;
        if dense {
            doc.renumber_objects();
        }
        doc.reference_table.cross_reference_type = if table {
            lopdf::xref::XrefType::CrossReferenceTable
        } else {
            lopdf::xref::XrefType::CrossReferenceStream
        };
        doc.trailer.set(
            "ID",
            Object::Array(vec![
                Object::String(vec![1; 16], lopdf::StringFormat::Hexadecimal),
                Object::String(vec![1; 16], lopdf::StringFormat::Hexadecimal),
            ]),
        );
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes)?;
        match signing::sign(&bytes, &signer, name, true).await {
            Ok(signed) => write_new(&out.join(format!("{name}.pdf")), &signed)?,
            Err(error) if !table => println!("Expected stream-input rejection: {error}"),
            Err(error) => return Err(error),
        }
    }
    let mut doc = Document::with_version("1.4");
    doc.reference_table.cross_reference_type = lopdf::xref::XrefType::CrossReferenceTable;
    let pages = doc.new_object_id();
    let stream = doc.add_object(Stream::new(Dictionary::new(), b"".to_vec()));
    let page=doc.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"MediaBox"=>vec![0.into(),0.into(),612.into(),792.into()],"Resources"=>Dictionary::new(),"Contents"=>stream});
    doc.objects.insert(
        pages,
        dictionary! {"Type"=>"Pages","Kids"=>vec![page.into()],"Count"=>1}.into(),
    );
    let root = doc.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages});
    doc.trailer.set("Root", root);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes)?;
    write_new(
        &out.join("simple-table.pdf"),
        &signing::sign(&bytes, &signer, "Simple", true).await?,
    )?;
    println!("Probe outputs: {}", out.display());
    Ok(())
}
