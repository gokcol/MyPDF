//! Reversible image framing and repeatable artwork; independent of desktop UI.
use crate::annotations::{Annotation, Box2, Kind};
use anyhow::{Result, ensure};
use std::sync::{Arc, Mutex, OnceLock};

/// Duplicate a selection without connecting its groups to the original objects.
pub fn duplicate(marks: &[Annotation]) -> Vec<Annotation> {
    let mut groups = std::collections::HashMap::new();
    marks
        .iter()
        .map(|a| {
            let mut copy = a.duplicate();
            copy.object.group = a.object.group.as_ref().map(|g| {
                groups
                    .entry((a.page, g.clone()))
                    .or_insert_with(|| format!("{:032x}", rand::random::<u128>()))
                    .clone()
            });
            copy
        })
        .collect()
}

/// Expand a clicked member to its visible siblings on the same page.
pub fn expand_groups(marks: &[Annotation], ids: &[String]) -> Vec<String> {
    let groups: Vec<_> = marks
        .iter()
        .filter(|a| ids.contains(&a.id))
        .filter_map(|a| a.object.group.as_ref().map(|g| (a.page, g)))
        .collect();
    marks
        .iter()
        .filter(|a| {
            ids.contains(&a.id)
                || (!a.object.hidden
                    && a.object
                        .group
                        .as_ref()
                        .is_some_and(|g| groups.contains(&(a.page, g))))
        })
        .map(|a| a.id.clone())
        .collect()
}

pub fn group(marks: &mut [Annotation], ids: &[String], ungroup: bool) -> Result<()> {
    let selected: Vec<_> = marks.iter().filter(|a| ids.contains(&a.id)).collect();
    ensure!(
        !selected.is_empty() && (ungroup || selected.len() > 1),
        "Select at least two objects to group"
    );
    ensure!(
        selected
            .iter()
            .all(|a| !a.object.locked && a.page == selected[0].page),
        "Unlock the objects and select them on one page"
    );
    let id = (!ungroup).then(|| format!("{:032x}", rand::random::<u128>()));
    for a in marks.iter_mut().filter(|a| ids.contains(&a.id)) {
        a.object.group = id.clone();
    }
    Ok(())
}

pub fn validate_crop(r: Box2) -> Result<()> {
    ensure!(
        r.iter().all(|v| v.is_finite() && (0.0..=1.).contains(v))
            && r[2] - r[0] >= 0.001
            && r[3] - r[1] >= 0.001,
        "Crop must stay inside the image"
    );
    Ok(())
}
pub fn image_appearance(a: &Annotation) -> Result<Arc<Vec<u8>>> {
    let Kind::Image { png } = &a.kind else {
        anyhow::bail!("Select an image");
    };
    crate::image_effects::blurred(&cropped(png, a.object.crop)?, a.style.image_blur)
}
type CropCache = Vec<(Arc<Vec<u8>>, Box2, Arc<Vec<u8>>)>;
pub fn cropped(png: &Arc<Vec<u8>>, crop: Option<Box2>) -> Result<Arc<Vec<u8>>> {
    let Some(r) = crop else {
        return Ok(png.clone());
    };
    validate_crop(r)?;
    static CACHE: OnceLock<Mutex<CropCache>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some((_, _, p)) = cache
        .lock()
        .unwrap()
        .iter()
        .find(|(source, crop, _)| Arc::ptr_eq(source, png) && crop == &r)
    {
        return Ok(p.clone());
    }
    let image = crate::assets::decode_png(png)?;
    let (w, h) = (image.width(), image.height());
    let x = (r[0] * w as f32).floor() as u32;
    let y = (r[1] * h as f32).floor() as u32;
    let right = ((r[2] * w as f32).ceil() as u32).clamp(x + 1, w);
    let bottom = ((r[3] * h as f32).ceil() as u32).clamp(y + 1, h);
    let image = image.crop_imm(x, y, right - x, bottom - y);
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png)?;
    ensure!(
        bytes.get_ref().len() <= crate::assets::MAX_ASSET_BYTES,
        "Cropped image is too large"
    );
    let result = Arc::new(bytes.into_inner());
    let mut cache = cache.lock().unwrap();
    while !cache.is_empty()
        && (cache.len() >= 8
            || cache
                .iter()
                .map(|(source, _, p)| source.len() + p.len())
                .sum::<usize>()
                + png.len()
                + result.len()
                > 32 * 1024 * 1024)
    {
        cache.remove(0);
    }
    cache.push((png.clone(), r, result.clone()));
    Ok(result)
}
/// Change the visible frame without moving the remaining pixels. Rotation/flip
/// are accounted for by transforming the center shift into page coordinates.
pub fn crop(a: &Annotation, next: Option<Box2>) -> Result<Annotation> {
    ensure!(matches!(a.kind, Kind::Image { .. }), "Select an image");
    let old = a.object.crop.unwrap_or([0., 0., 1., 1.]);
    let r = next.unwrap_or([0., 0., 1., 1.]);
    validate_crop(r)?;
    let full_w = (a.rect[2] - a.rect[0]) / (old[2] - old[0]);
    let full_h = (a.rect[3] - a.rect[1]) / (old[3] - old[1]);
    let center = [(a.rect[0] + a.rect[2]) / 2., (a.rect[1] + a.rect[3]) / 2.];
    let local = [
        center[0] + (r[0] + r[2] - old[0] - old[2]) * full_w / 2.,
        center[1] + (r[1] + r[3] - old[1] - old[3]) * full_h / 2.,
    ];
    let c = a.rotated(local);
    let (w, h) = ((r[2] - r[0]) * full_w, (r[3] - r[1]) * full_h);
    ensure!(
        w >= 1. && h >= 1.,
        "Crop must be at least one point wide and high"
    );
    let mut result = a.clone();
    result.rect = [c[0] - w / 2., c[1] - h / 2., c[0] + w / 2., c[1] + h / 2.];
    result.object.crop = next;
    Ok(result)
}
/// Detect transparent or near-white outer margins, never interior white areas.
pub fn trim(png: &[u8]) -> Result<Box2> {
    let image = crate::assets::decode_png(png)?.into_rgba8();
    let (w, h) = image.dimensions();
    let mut b = [w, h, 0, 0];
    for (x, y, p) in image.enumerate_pixels() {
        if p[3] > 12 && p.0[..3].iter().any(|c| *c < 245) {
            b = [b[0].min(x), b[1].min(y), b[2].max(x + 1), b[3].max(y + 1)];
        }
    }
    ensure!(
        b[2] > b[0] && b[3] > b[1],
        "No artwork found inside the white or transparent margins"
    );
    Ok([
        b[0] as f32 / w as f32,
        b[1] as f32 / h as f32,
        b[2] as f32 / w as f32,
        b[3] as f32 / h as f32,
    ])
}
/// Keep position on equally sized pages, fitting the entire group on smaller pages.
pub fn repeat(marks: &[Annotation], pages: &[(usize, [f32; 2])]) -> Vec<Annotation> {
    let mut result = vec![];
    if marks.is_empty() {
        return result;
    }
    let b = crate::annotations::bounds(
        &marks
            .iter()
            .flat_map(|a| {
                let b = a.bounds();
                [[b[0], b[1]], [b[2], b[3]]]
            })
            .collect::<Vec<_>>(),
    );
    for &(page, size) in pages {
        if marks.iter().all(|a| a.page == page) {
            continue;
        }
        let scale = (size[0] / (b[2] - b[0]))
            .min(size[1] / (b[3] - b[1]))
            .min(1.);
        let x = b[0].clamp(0., (size[0] - (b[2] - b[0]) * scale).max(0.));
        let y = b[1].clamp(0., (size[1] - (b[3] - b[1]) * scale).max(0.));
        for (a, mut c) in marks.iter().zip(duplicate(marks)) {
            c.page = page;
            c.rect = [
                x + (a.rect[0] - b[0]) * scale,
                y + (a.rect[1] - b[1]) * scale,
                x + (a.rect[2] - b[0]) * scale,
                y + (a.rect[3] - b[1]) * scale,
            ];
            c.style.font_size = (c.style.font_size * scale).max(1.);
            result.push(c);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotations::{self as ann, Session};
    fn image() -> Annotation {
        let image = image::RgbaImage::from_fn(100, 80, |x, y| {
            if (20..80).contains(&x) && (10..60).contains(&y) {
                image::Rgba([20, 120, 190, 255])
            } else {
                image::Rgba([255, 255, 255, 255])
            }
        });
        let mut bytes = std::io::Cursor::new(vec![]);
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        Annotation::new(
            0,
            [60., 90., 260., 250.],
            Kind::Image {
                png: Arc::new(bytes.into_inner()),
            },
            Default::default(),
        )
    }
    #[test]
    fn crop_retains_source_and_restores_after_rotation_flip_save_and_reopen() {
        for rotation in [0., 90., -37.] {
            for flip in [false, true] {
                let mut a = image();
                a.rotation = rotation;
                a.flip_x = flip;
                let b = crop(&a, Some([0.2, 0.125, 0.8, 0.75])).unwrap();
                assert_eq!(b.kind, a.kind);
                let raster = crate::assets::decode_png(&image_appearance(&b).unwrap()).unwrap();
                assert_eq!((raster.width(), raster.height()), (60, 50));
                let source = crate::convert::blank(400., 400.).unwrap();
                let bytes = ann::export(&source, std::slice::from_ref(&b)).unwrap();
                let loaded = ann::load(&lopdf::Document::load_mem(&bytes).unwrap());
                assert_eq!(loaded, vec![b]);
                let restored = crop(&loaded[0], None).unwrap();
                for (x, y) in restored.rect.iter().zip(a.rect) {
                    assert!((x - y).abs() < 0.001);
                }
                assert_eq!(restored.kind, a.kind);
                assert_eq!(restored.object.crop, None);
            }
        }
    }
    #[test]
    fn trim_only_crops_outer_margins_and_validates_inputs() {
        let a = image();
        let Kind::Image { png } = &a.kind else {
            unreachable!()
        };
        assert_eq!(trim(png).unwrap(), [0.2, 0.125, 0.8, 0.75]);
        for r in [[0., 0., 0., 1.], [-1., 0., 1., 1.], [0., 0., f32::NAN, 1.]] {
            assert!(crop(&a, Some(r)).is_err());
        }
    }
    #[test]
    fn visibility_lock_name_survive_save_and_locked_edits_are_atomic() {
        let source: Arc<[u8]> = crate::convert::blank(400., 400.).unwrap().into();
        let mut a = image();
        a.object.name = "Company seal".into();
        a.object.hidden = true;
        a.object.locked = true;
        assert!(ann::primitives(&a).is_empty());
        assert!(!a.hit_test([80., 100.], 5.));
        let bytes = ann::export(&source, std::slice::from_ref(&a)).unwrap();
        let pdf = lopdf::Document::load_mem(&bytes).unwrap();
        assert_eq!(ann::load(&pdf), vec![a.clone()]);
        let mut s = Session::new(source, vec![a.clone()], true);
        assert!(s.commit(vec![]).is_err());
        let mut moved = a.clone();
        moved.rect[0] += 3.;
        assert!(s.commit(vec![moved]).is_err());
        assert_eq!(s.revision(), 0);
        a.object.hidden = false;
        s.commit(vec![a.clone()]).unwrap();
        assert_eq!(s.undo_label().as_deref(), Some("Show image"));
        a.object.locked = false;
        s.commit(vec![a.clone()]).unwrap();
        a.rect[0] += 3.;
        s.commit(vec![a]).unwrap();
        s.seek_history(0);
        assert!(s.annotations[0].object.locked);
        assert!(s.annotations[0].object.hidden);
        s.seek_history(3);
        assert!(!s.annotations[0].object.locked);
        assert_eq!(s.annotations[0].rect[0], 63.);
    }
    #[test]
    fn repeat_uses_unique_ids_fits_pages_and_undoes_in_one_step() {
        let a = image();
        let copies = repeat(
            std::slice::from_ref(&a),
            &[(0, [400., 400.]), (1, [400., 400.]), (2, [100., 100.])],
        );
        assert_eq!(copies.len(), 2);
        assert_eq!(copies[0].rect, a.rect);
        assert_ne!(copies[0].id, copies[1].id);
        let b = copies[1].bounds();
        assert!(b[0] >= 0. && b[1] >= 0. && b[2] <= 100.001 && b[3] <= 100.001);
        let mut s = Session::new(vec![].into(), vec![a.clone()], true);
        let mut all = vec![a.clone()];
        all.extend(copies);
        s.commit(all.clone()).unwrap();
        s.name_last_change("Repeat artwork on pages");
        assert_eq!(s.undo_label().as_deref(), Some("Repeat artwork on pages"));
        s.undo();
        assert_eq!(s.annotations, vec![a]);
        assert_eq!(s.redo_label().as_deref(), Some("Repeat artwork on pages"));
        s.redo();
        assert_eq!(s.annotations, all);
        let (cursor, names) = s.history();
        assert_eq!(cursor, 1);
        assert_eq!(names[1], "Repeat artwork on pages");
    }
    #[test]
    fn legacy_metadata_and_coalesced_history_remain_compatible() {
        let a = image();
        let mut json = serde_json::to_value(&a).unwrap();
        json.as_object_mut().unwrap().remove("object");
        let old: Annotation = serde_json::from_value(json).unwrap();
        assert_eq!(old.object, Default::default());
        let mut s = Session::new(vec![].into(), vec![a], true);
        let mut entries = s.annotations.clone();
        entries[0].style.opacity = 0.8;
        s.commit_live(entries.clone(), None).unwrap();
        let rev = s.revision();
        entries[0].style.opacity = 0.5;
        s.commit_live(entries, Some(rev)).unwrap();
        assert_eq!(s.history().0, 1);
        s.undo();
        assert_eq!(s.annotations[0].style.opacity, 1.);
        s.redo();
        assert_eq!(s.annotations[0].style.opacity, 0.5);
        s.undo();
        let mut entries = s.annotations.clone();
        entries[0].object.name = "Seal".into();
        s.commit(entries).unwrap();
        assert!(!s.can_redo());
        assert_eq!(s.undo_label().as_deref(), Some("Rename image"));
    }
}
