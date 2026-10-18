//! Small JSON boundary for the native iPad shell. PDF changes use the shared Rust engine.
use anyhow::{Context, Result, ensure};
use mypdf_milestone0::{annotations as ann, image_background};
use serde_json::{Value, json};
use std::{
    ffi::{CStr, CString, c_char},
    path::Path,
    sync::Arc,
};
mod page_commands;

fn path<'a>(v: &'a Value, key: &str) -> Result<&'a Path> {
    Ok(Path::new(
        v[key].as_str().with_context(|| format!("Missing {key}"))?,
    ))
}
fn array<const N: usize>(v: &Value) -> Result<[f32; N]> {
    let values = v.as_array().context("Expected coordinates")?;
    ensure!(values.len() == N, "Wrong coordinate count");
    let mut out = [0.; N];
    for (i, value) in values.iter().enumerate() {
        out[i] = value.as_f64().context("Invalid coordinate")? as f32;
        ensure!(out[i].is_finite(), "Invalid coordinate");
    }
    Ok(out)
}
fn perform(v: Value) -> Result<Value> {
    let command = v["command"].as_str().context("Missing command")?;
    if command == "blank" {
        mypdf_milestone0::viewer::save_copy(
            path(&v, "output")?,
            &mypdf_milestone0::convert::blank(595., 842.)?,
        )?;
        return Ok(json!({}));
    }
    if command == "background" {
        let input = mypdf_milestone0::convert::read_bounded(
            path(&v, "input")?,
            mypdf_milestone0::assets::MAX_ASSET_BYTES as u64,
        )?;
        let options = image_background::Options {
            tolerance: v["tolerance"].as_u64().unwrap_or(24).min(120) as u8,
            edges_only: v["edgesOnly"].as_bool().unwrap_or(false),
            ..Default::default()
        };
        mypdf_milestone0::viewer::save_copy(
            path(&v, "output")?,
            &image_background::remove(&input, options)?,
        )?;
        return Ok(json!({}));
    }
    let bytes = mypdf_milestone0::convert::read_bounded(path(&v, "input")?, 256 * 1024 * 1024)?;
    if command == "pages" {
        return page_commands::perform(&bytes, &v);
    }
    let document = lopdf::Document::load_mem(&bytes)?;
    let mut marks = ann::load(&document);
    if command == "annotations" {
        return Ok(
            json!({"annotations":marks,"pages":ann::page_geometry(&bytes)?.into_iter().map(|(r,_)|[r[2],r[3]]).collect::<Vec<_>>(),"editable":ann::inspect(&document).rewritable()}),
        );
    }
    ensure!(
        ann::inspect(&document).rewritable(),
        "Signed, certified, or encrypted PDFs are read-only in this build"
    );
    let page = v["page"].as_u64().unwrap_or(0) as usize;
    ensure!(page < document.get_pages().len(), "Page not found");
    let mut added = None;
    match command {
        "ink" => {
            let strokes: Vec<Vec<[f32; 2]>> = serde_json::from_value(v["strokes"].clone())?;
            ensure!(
                !strokes.is_empty() && strokes.len() <= 2000,
                "No drawing or too many strokes"
            );
            let points: Vec<_> = strokes.iter().flatten().copied().collect();
            ensure!(
                points.len() >= 2
                    && points.len() <= 100_000
                    && points.iter().flatten().all(|p| p.is_finite()),
                "Invalid drawing"
            );
            let mut rect = ann::bounds(&points);
            rect[2] = rect[2].max(rect[0] + 1.);
            rect[3] = rect[3].max(rect[1] + 1.);
            let strokes = strokes
                .iter()
                .map(|stroke| {
                    stroke
                        .iter()
                        .map(|p| {
                            [
                                (p[0] - rect[0]) / (rect[2] - rect[0]),
                                (p[1] - rect[1]) / (rect[3] - rect[1]),
                            ]
                        })
                        .collect()
                })
                .collect();
            let style = ann::Style {
                color: array(&v["color"])?,
                width: v["width"].as_f64().unwrap_or(2.) as f32,
                ..Default::default()
            };
            let mark = ann::Annotation::new(page, rect, ann::Kind::Ink { strokes }, style);
            added = Some(mark.id.clone());
            marks.push(mark);
        }
        "image" => {
            let png = mypdf_milestone0::assets::import_artwork(path(&v, "image")?)?;
            let mark = ann::Annotation::new(
                page,
                array(&v["rect"])?,
                ann::Kind::Image { png },
                ann::Style::default(),
            );
            added = Some(mark.id.clone());
            marks.push(mark);
        }
        "place" | "delete" | "transparent" | "preview_without" | "replace_image" => {
            let id = v["id"].as_str().context("Select an object")?;
            let index = marks
                .iter()
                .position(|m| m.id == id && m.page == page)
                .context("Object not found")?;
            ensure!(
                command == "preview_without" || !marks[index].object.locked,
                "Unlock the object before editing it"
            );
            if command == "delete" || command == "preview_without" {
                marks.remove(index);
            } else if command == "place" {
                marks[index].rect = array(&v["rect"])?;
            } else if command == "replace_image" {
                marks[index].kind = ann::Kind::Image {
                    png: mypdf_milestone0::assets::import_artwork(path(&v, "image")?)?,
                };
            } else {
                let ann::Kind::Image { png } = &marks[index].kind else {
                    anyhow::bail!("Select an image or image signature")
                };
                let options = image_background::Options {
                    tolerance: v["tolerance"].as_u64().unwrap_or(24).min(120) as u8,
                    ..Default::default()
                };
                marks[index].kind = ann::Kind::Image {
                    png: Arc::new(image_background::remove(png, options)?),
                };
            }
        }
        _ => anyhow::bail!("Unsupported operation"),
    }
    ensure!(marks.len() <= 5000, "Too many objects");
    for mark in &marks {
        mark.validate()?;
    }
    let output = ann::export(&bytes, &marks)?;
    mypdf_milestone0::viewer::save_copy(path(&v, "output")?, &output)?;
    Ok(json!({"annotations":marks,"selected":added}))
}

/// Input is a UTF-8, NUL-terminated JSON string owned by the caller.
/// The returned JSON string must be released with mypdf_ipad_free.
/// # Safety
/// `input` must point to a valid NUL-terminated string for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mypdf_ipad_call(input: *const c_char) -> *mut c_char {
    let result = std::panic::catch_unwind(|| -> Result<Value> {
        ensure!(!input.is_null(), "Missing request");
        let text = unsafe { CStr::from_ptr(input) }.to_str()?;
        ensure!(text.len() < 1024 * 1024, "Request is too large");
        perform(serde_json::from_str(text)?)
    });
    let response = match result {
        Ok(Ok(value)) => json!({"ok":true,"result":value}),
        Ok(Err(error)) => json!({"ok":false,"error":format!("{error:#}")}),
        Err(_) => {
            json!({"ok":false,"error":"The operation could not be completed. The source is unchanged."})
        }
    };
    CString::new(response.to_string()).unwrap().into_raw()
}
#[unsafe(no_mangle)]
/// # Safety
/// Pass only a non-freed pointer returned by `mypdf_ipad_call`, or null.
pub unsafe extern "C" fn mypdf_ipad_free(value: *mut c_char) {
    if !value.is_null() {
        drop(unsafe { CString::from_raw(value) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mypdf_milestone0::{convert, object_tools, review};
    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn desktop_objects_and_review_metadata_survive_ipad_edit_and_page_round_trip() {
        let ink = ann::Annotation::new(
            0,
            [30., 40., 150., 100.],
            ann::Kind::Ink {
                strokes: vec![vec![[0., 0.], [0.5, 1.], [1., 0.]]],
            },
            Default::default(),
        );
        let root = Scratch(std::env::temp_dir().join(format!("mypdf-ipad-roundtrip-{}", ink.id)));
        std::fs::create_dir(&root.0).unwrap();
        let mut objects = vec![
            ink.clone(),
            ann::Annotation::new(
                0,
                [200., 40., 300., 120.],
                ann::Kind::Rectangle,
                Default::default(),
            ),
            ann::Annotation::new(
                0,
                [220., 140., 320., 190.],
                ann::Kind::Note {
                    text: "Check layout".into(),
                },
                Default::default(),
            ),
        ];
        let ids = objects[1..]
            .iter()
            .map(|a| a.id.clone())
            .collect::<Vec<_>>();
        object_tools::group(&mut objects, &ids, false).unwrap();
        objects[2].object.review = review::Review {
            author: "Reviewer".into(),
            created: 1,
            resolved: true,
            replies: vec![review::Reply {
                author: "Editor".into(),
                created: 2,
                text: "Corrected".into(),
            }],
        };
        let input = root.0.join("desktop.pdf");
        let edited = root.0.join("ipad.pdf");
        let paged = root.0.join("pages.pdf");
        let source = convert::blank(500., 700.).unwrap();
        let saved = ann::export(&source, &objects).unwrap();
        std::fs::write(&input, &saved).unwrap();
        perform(json!({"command":"place","input":input,"output":edited,"page":0,"id":ink.id,"rect":[70.,80.,190.,140.]})).unwrap();
        objects[0].rect = [70., 80., 190., 140.];
        let loaded = perform(json!({"command":"annotations","input":edited})).unwrap();
        assert_eq!(
            loaded["annotations"],
            serde_json::to_value(&objects).unwrap()
        );
        perform(json!({"command":"pages","action":"blank","input":edited,"output":paged,"at":0,"size":[500.,700.]})).unwrap();
        for a in &mut objects {
            a.page = 1;
        }
        let doc = lopdf::Document::load(&paged).unwrap();
        assert_eq!(ann::load(&doc), objects);
        assert_eq!(
            std::fs::read(&input).unwrap(),
            saved,
            "Source file must stay unchanged"
        );
        objects[0].object.locked = true;
        let locked = root.0.join("locked.pdf");
        let refused = root.0.join("refused.pdf");
        std::fs::write(
            &locked,
            ann::export(&std::fs::read(&paged).unwrap(), &objects).unwrap(),
        )
        .unwrap();
        assert!(
            perform(
                json!({"command":"delete","input":locked,"output":refused,"page":1,"id":ink.id})
            )
            .is_err()
        );
        assert!(!refused.exists());
    }
}
