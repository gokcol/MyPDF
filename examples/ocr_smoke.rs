//! CI smoke check: a real rendered scan must become searchable using the bundled backend.
use anyhow::{Result, ensure};
use mypdf_milestone0::{document, ocr, redact, viewer};
fn main() -> Result<()> {
    let font = document::default_font()?;
    let original = document::fixture(&font)?;
    let (scan, _) = redact::flatten_document(&original, 200.)?;
    let (result, report) = ocr::recognize(&scan, &|done, total| println!("OCR {done}/{total}"))?;
    ensure!(
        report.pages == 2 && report.text.len() == 2,
        "Expected results from both scanned pages"
    );
    ensure!(
        report.text[0].1.to_lowercase().contains("invoice"),
        "Missing recognized invoice text: {:?}",
        report.text
    );
    let pdf = hayro::hayro_syntax::Pdf::new(result).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let text = viewer::extract_page(
        &pdf.pages()[0],
        &hayro::hayro_interpret::InterpreterCache::new(),
    );
    ensure!(
        text.text.to_lowercase().contains("invoice"),
        "Saved OCR layer is not searchable"
    );
    let cancelled = ocr::Cancellation::default();
    cancelled.cancel();
    ensure!(
        ocr::recognize_cancellable(&scan, &cancelled, &|_| {}).is_err(),
        "Cancelled OCR must not produce a document"
    );
    println!("PASS real scan recognition, results text, searchable PDF layer, and cancellation");
    Ok(())
}
