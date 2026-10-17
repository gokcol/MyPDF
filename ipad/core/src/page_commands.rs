//! iPad page commands delegate to the same transactional operations as desktop.
use super::*;
use mypdf_milestone0::pages::{self, Operation};

fn index(v: &Value, key: &str) -> Result<usize> {
    usize::try_from(v[key].as_u64().with_context(|| format!("Missing {key}"))?)
        .context("Invalid page index")
}
fn selection(v: &Value, key: &str) -> Result<Vec<usize>> {
    let values = v[key]
        .as_array()
        .with_context(|| format!("Missing {key}"))?;
    ensure!(
        !values.is_empty() && values.len() <= 10_000,
        "Select 1–10,000 pages"
    );
    values
        .iter()
        .map(|v| {
            usize::try_from(v.as_u64().context("Invalid page index")?).context("Invalid page index")
        })
        .collect()
}
pub(super) fn perform(bytes: &[u8], v: &Value) -> Result<Value> {
    let action = v["action"].as_str().context("Choose a page operation")?;
    let output = match action {
        "extract" => pages::extract(bytes, &selection(v, "pages")?)?,
        _ => {
            let operation = match action {
                "reorder" => Operation::Reorder(selection(v, "order")?),
                "delete" => Operation::Delete(selection(v, "pages")?),
                "blank" => {
                    let size = array::<2>(&v["size"])?;
                    Operation::InsertBlank {
                        at: index(v, "at")?,
                        width: size[0],
                        height: size[1],
                    }
                }
                "insert" => Operation::InsertPdf {
                    at: index(v, "at")?,
                    bytes: mypdf_milestone0::convert::read_bounded(
                        path(v, "addition")?,
                        256 * 1024 * 1024,
                    )?,
                },
                "rotate" => Operation::Rotate {
                    pages: selection(v, "pages")?,
                    clockwise: v["clockwise"]
                        .as_bool()
                        .context("Choose a rotation direction")?,
                },
                "crop" => Operation::Crop {
                    pages: selection(v, "pages")?,
                    margins: array(&v["margins"])?,
                },
                "reset_crop" => Operation::ResetCrop(selection(v, "pages")?),
                _ => anyhow::bail!("Unknown page operation"),
            };
            pages::apply(bytes, &operation)?
        }
    };
    mypdf_milestone0::viewer::save_copy(path(v, "output")?, &output)?;
    Ok(json!({"page_count":lopdf::Document::load_mem(&output)?.get_pages().len()}))
}
