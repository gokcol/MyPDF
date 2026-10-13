//! Local reusable artwork. No passwords or private keys belong in this store.
use crate::annotations::{Kind, Style};
use anyhow::{Context, Result, ensure};
use image::{DynamicImage, ImageFormat, ImageReader};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

pub const MAX_ASSET_BYTES: usize = 8 * 1024 * 1024;
const MAX_LIBRARY_BYTES: u64 = 64 * 1024 * 1024;
pub fn data_directory() -> PathBuf {
    if let Some(path) = std::env::var_os("MYPDF_DATA_DIR") {
        return PathBuf::from(path);
    }
    #[cfg(target_os = "macos")]
    return PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
        .join("Library/Application Support/MyPDF");
    #[cfg(target_os = "windows")]
    return PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("MyPDF");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    return PathBuf::from(std::env::var_os("XDG_DATA_HOME").unwrap_or_else(|| {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
            .join(".local/share")
            .into_os_string()
    }))
    .join("mypdf");
}
pub fn decode_png(bytes: &[u8]) -> Result<DynamicImage> {
    ensure!(
        bytes.len() <= MAX_ASSET_BYTES,
        "Artwork exceeds the 8 MB limit"
    );
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    reader
        .decode()
        .context("Could not decode artwork (PNG/JPEG, up to 4096 × 4096 pixels)")
}
pub fn import_artwork(path: &Path) -> Result<Arc<Vec<u8>>> {
    let mut bytes = vec![];
    std::fs::File::open(path)?
        .take(MAX_ASSET_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_ASSET_BYTES,
        "Artwork exceeds the 8 MB limit"
    );
    let image = if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("svg"))
    {
        let mut options = resvg::usvg::Options::default();
        // Only embedded data images are accepted. SVG scripts, network, and file references are not executed/resolved.
        let external = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = external.clone();
        options.image_href_resolver.resolve_string = Box::new(move |_, _| {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
            None
        });
        options.image_href_resolver.resolve_data = Box::new(|_, data, _| {
            decode_png(&data).ok()?;
            match image::guess_format(&data).ok()? {
                ImageFormat::Png => Some(resvg::usvg::ImageKind::PNG(data)),
                ImageFormat::Jpeg => Some(resvg::usvg::ImageKind::JPEG(data)),
                _ => None,
            }
        });
        options.fontdb_mut().load_system_fonts();
        let tree = resvg::usvg::Tree::from_str(
            std::str::from_utf8(&bytes).context("SVG artwork must be uncompressed UTF-8")?,
            &options,
        )
        .context("Could not read SVG artwork")?;
        ensure!(
            !external.load(std::sync::atomic::Ordering::Relaxed),
            "SVG artwork references an external image. Embed its image data before importing."
        );
        let size = tree.size();
        let scale = (2048.0 / size.width().max(size.height())).min(1.0);
        let w = (size.width() * scale).ceil().max(1.0) as u32;
        let h = (size.height() * scale).ceil().max(1.0) as u32;
        let mut pixmap =
            resvg::tiny_skia::Pixmap::new(w, h).context("Could not allocate artwork preview")?;
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(scale, scale),
            &mut pixmap.as_mut(),
        );
        let png = pixmap.encode_png()?;
        decode_png(&png)?
    } else {
        decode_png(&bytes)?
    };
    let image = if image.width().max(image.height()) > 2048 {
        image.thumbnail(2048, 2048)
    } else {
        image
    };
    let mut out = Cursor::new(vec![]);
    image.write_to(&mut out, ImageFormat::Png)?;
    let out = out.into_inner();
    ensure!(
        out.len() <= MAX_ASSET_BYTES,
        "Normalized artwork exceeds the 8 MB limit"
    );
    Ok(Arc::new(out))
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Asset {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub style: Style,
    pub aspect: f32,
    #[serde(default)]
    pub stamp: bool,
}
impl Asset {
    pub fn new(name: &str, kind: Kind, style: Style, aspect: f32) -> Self {
        Self {
            id: format!("{:032x}", rand::random::<u128>()),
            name: name.into(),
            kind,
            style,
            aspect,
            stamp: false,
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.name.trim().is_empty()
                && self.name.len() <= 200
                && self.id.len() == 32
                && self.id.bytes().all(|b| b.is_ascii_hexdigit())
                && self.aspect.is_finite()
                && (0.02..=50.0).contains(&self.aspect),
            "Invalid asset details"
        );
        self.kind.validate()?;
        crate::annotations::Annotation::new(
            0,
            [0.0, 0.0, 100.0, 100.0],
            self.kind.clone(),
            self.style.clone(),
        )
        .validate()
    }
}

/// Built-in vector artwork is available without changing the user's local library.
/// Stable IDs keep previews and placement independent of catalogue writes.
pub struct BuiltinAsset {
    pub category: &'static str,
    pub asset: Asset,
}
pub fn builtin_assets() -> Vec<BuiltinAsset> {
    use crate::annotations::Point2;
    let blue = [0.12, 0.46, 0.83];
    let green = [0.07, 0.62, 0.39];
    let red = [0.94, 0.27, 0.28];
    let gold = [0.96, 0.68, 0.08];
    let purple = [0.48, 0.32, 0.73];
    let ink = [0.18, 0.20, 0.25];
    let mut out = vec![];
    let mut add = |category, name: &str, polygons: Vec<Vec<Point2>>, color, aspect| {
        let mut asset = Asset::new(
            name,
            Kind::Vector { polygons },
            Style {
                color,
                width: 2.0,
                ..Default::default()
            },
            aspect,
        );
        asset.id = format!("b27a{:028x}", out.len());
        out.push(BuiltinAsset { category, asset });
    };
    let arrow = vec![
        [0.06, 0.34],
        [0.61, 0.34],
        [0.61, 0.08],
        [0.97, 0.5],
        [0.61, 0.92],
        [0.61, 0.66],
        [0.06, 0.66],
    ];
    for (name, color) in [
        ("Arrow right", blue),
        ("Arrow coral", red),
        ("Arrow gold", gold),
        ("Arrow violet", purple),
    ] {
        add("Arrows", name, vec![arrow.clone()], color, 1.9);
    }
    add(
        "Arrows",
        "Arrow left",
        vec![arrow.iter().map(|p| [1.0 - p[0], p[1]]).collect()],
        blue,
        1.9,
    );
    add(
        "Arrows",
        "Arrow up",
        vec![arrow.iter().map(|p| [p[1], 1.0 - p[0]]).collect()],
        blue,
        0.53,
    );
    add(
        "Arrows",
        "Arrow down",
        vec![arrow.iter().map(|p| [p[1], p[0]]).collect()],
        blue,
        0.53,
    );
    add(
        "Arrows",
        "Double arrow",
        vec![vec![
            [0.03, 0.5],
            [0.29, 0.1],
            [0.29, 0.35],
            [0.71, 0.35],
            [0.71, 0.1],
            [0.97, 0.5],
            [0.71, 0.9],
            [0.71, 0.65],
            [0.29, 0.65],
            [0.29, 0.9],
        ]],
        purple,
        2.1,
    );
    for (name, angle) in [
        ("Arrow northeast", -45.0f32),
        ("Arrow southeast", 45.0),
        ("Arrow northwest", -135.0),
        ("Arrow southwest", 135.0),
    ] {
        let (sn, cs) = angle.to_radians().sin_cos();
        add(
            "Arrows",
            name,
            vec![
                arrow
                    .iter()
                    .map(|p| {
                        let x = (p[0] - 0.5) * 0.88;
                        let y = (p[1] - 0.5) * 0.46;
                        [0.5 + x * cs - y * sn, 0.5 + x * sn + y * cs]
                    })
                    .collect(),
            ],
            ink,
            1.0,
        );
    }
    add(
        "Arrows",
        "Elbow arrow",
        vec![vec![
            [0.08, 0.08],
            [0.28, 0.08],
            [0.28, 0.57],
            [0.63, 0.57],
            [0.63, 0.33],
            [0.96, 0.68],
            [0.63, 0.97],
            [0.63, 0.77],
            [0.08, 0.77],
        ]],
        blue,
        1.2,
    );
    add(
        "Arrows",
        "Chevron right",
        vec![vec![
            [0.2, 0.1],
            [0.43, 0.1],
            [0.83, 0.5],
            [0.43, 0.9],
            [0.2, 0.9],
            [0.6, 0.5],
        ]],
        ink,
        0.75,
    );
    add(
        "Arrows",
        "Chevron left",
        vec![vec![
            [0.8, 0.1],
            [0.57, 0.1],
            [0.17, 0.5],
            [0.57, 0.9],
            [0.8, 0.9],
            [0.4, 0.5],
        ]],
        ink,
        0.75,
    );
    let check = vec![
        [0.07, 0.48],
        [0.22, 0.36],
        [0.4, 0.57],
        [0.86, 0.12],
        [0.96, 0.2],
        [0.39, 0.88],
    ];
    for (name, color) in [
        ("Checkmark", green),
        ("Check blue", blue),
        ("Check gold", gold),
        ("Check charcoal", ink),
    ] {
        add("Symbols", name, vec![check.clone()], color, 1.12);
    }
    let cross = vec![
        [0.15, 0.05],
        [0.5, 0.4],
        [0.85, 0.05],
        [0.95, 0.15],
        [0.6, 0.5],
        [0.95, 0.85],
        [0.85, 0.95],
        [0.5, 0.6],
        [0.15, 0.95],
        [0.05, 0.85],
        [0.4, 0.5],
        [0.05, 0.15],
    ];
    add("Symbols", "Cross", vec![cross.clone()], red, 1.0);
    add("Symbols", "Cross charcoal", vec![cross], ink, 1.0);
    let ring = |radius: f32, width: f32| -> Vec<Vec<Point2>> {
        (0..64)
            .map(|i| {
                let point = |j: usize, r: f32| {
                    let angle = j as f32 / 64.0 * std::f32::consts::TAU;
                    [0.5 + r * angle.cos(), 0.5 + r * angle.sin()]
                };
                vec![
                    point(i, radius),
                    point(i + 1, radius),
                    point(i + 1, radius - width),
                    point(i, radius - width),
                ]
            })
            .collect()
    };
    add("Symbols", "Circle", ring(0.43, 0.085), blue, 1.0);
    let star: Vec<_> = (0..10)
        .map(|i| {
            let angle = i as f32 * std::f32::consts::PI / 5.0 - std::f32::consts::FRAC_PI_2;
            let r = if i % 2 == 0 { 0.46 } else { 0.21 };
            [0.5 + r * angle.cos(), 0.5 + r * angle.sin()]
        })
        .collect();
    add("Symbols", "Star", vec![star], gold, 1.0);
    let exclamation = vec![
        vec![[0.455, 0.31], [0.545, 0.31], [0.53, 0.59], [0.47, 0.59]],
        vec![[0.455, 0.65], [0.545, 0.65], [0.545, 0.74], [0.455, 0.74]],
    ];
    let mut warning = vec![
        vec![[0.5, 0.03], [0.98, 0.91], [0.85, 0.83], [0.5, 0.2]],
        vec![[0.02, 0.91], [0.5, 0.03], [0.5, 0.2], [0.15, 0.83]],
        vec![[0.02, 0.91], [0.15, 0.8], [0.85, 0.8], [0.98, 0.91]],
    ];
    warning.extend(exclamation.clone());
    add("Symbols", "Warning", warning, gold, 1.0);
    let mut attention = ring(0.44, 0.075);
    attention.extend(exclamation);
    add("Symbols", "Attention", attention, red, 1.0);
    add(
        "Symbols",
        "Plus",
        vec![vec![
            [0.42, 0.08],
            [0.58, 0.08],
            [0.58, 0.42],
            [0.92, 0.42],
            [0.92, 0.58],
            [0.58, 0.58],
            [0.58, 0.92],
            [0.42, 0.92],
            [0.42, 0.58],
            [0.08, 0.58],
            [0.08, 0.42],
            [0.42, 0.42],
        ]],
        green,
        1.0,
    );
    add(
        "Symbols",
        "Minus",
        vec![vec![[0.08, 0.4], [0.92, 0.4], [0.92, 0.6], [0.08, 0.6]]],
        red,
        1.0,
    );
    add(
        "Symbols",
        "Diamond",
        vec![vec![[0.5, 0.05], [0.94, 0.5], [0.5, 0.95], [0.06, 0.5]]],
        purple,
        1.0,
    );
    let mut info = ring(0.44, 0.075);
    info.extend([
        vec![[0.46, 0.25], [0.54, 0.25], [0.54, 0.34], [0.46, 0.34]],
        vec![[0.46, 0.42], [0.54, 0.42], [0.54, 0.75], [0.46, 0.75]],
    ]);
    add("Symbols", "Information", info, blue, 1.0);
    for (name, text, color) in [
        ("Approved", "APPROVED", green),
        ("Reviewed", "REVIEWED", blue),
        ("Draft", "DRAFT", gold),
        ("Confidential", "CONFIDENTIAL", red),
    ] {
        let mut asset = Asset::new(
            name,
            Kind::Stamp { text: text.into() },
            Style {
                color,
                width: 2.0,
                font_size: 18.0,
                ..Default::default()
            },
            3.8,
        );
        asset.id = format!("b27a{:028x}", out.len());
        out.push(BuiltinAsset {
            category: "Stamps",
            asset,
        });
    }
    out
}
pub struct Library {
    pub entries: Vec<Asset>,
    root: PathBuf,
    fingerprint: Vec<u8>,
}
impl Library {
    pub fn open(root: PathBuf) -> Result<Self> {
        let path = root.join("assets.json");
        let raw = match std::fs::File::open(&path) {
            Ok(file) => {
                let mut raw = vec![];
                file.take(MAX_LIBRARY_BYTES + 1).read_to_end(&mut raw)?;
                ensure!(
                    raw.len() as u64 <= MAX_LIBRARY_BYTES,
                    "Asset library exceeds its 64 MB limit"
                );
                raw
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![],
            Err(e) => return Err(e.into()),
        };
        let entries: Vec<Asset> = if raw.is_empty() {
            vec![]
        } else {
            serde_json::from_slice(&raw)
                .context("Could not read the asset library. Its original file is preserved.")?
        };
        ensure!(
            entries.len() <= 200,
            "Asset library contains too many items"
        );
        for entry in &entries {
            entry.validate()?;
        }
        Ok(Self {
            entries,
            root,
            fingerprint: Sha256::digest(raw).to_vec(),
        })
    }
    /// Replace by stable ID, preserving the original on validation or disk failure.
    pub fn put(&mut self, asset: Asset) -> Result<()> {
        asset.validate()?;
        let before = self.entries.clone();
        if let Some(entry) = self.entries.iter_mut().find(|a| a.id == asset.id) {
            *entry = asset;
        } else {
            self.entries.push(asset);
        }
        if let Err(e) = self.save() {
            self.entries = before;
            return Err(e);
        }
        Ok(())
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn save(&mut self) -> Result<()> {
        ensure!(
            self.entries.len() <= 200,
            "The library supports up to 200 items"
        );
        for entry in &self.entries {
            entry.validate()?;
        }
        let raw = serde_json::to_vec(&self.entries)?;
        ensure!(
            raw.len() as u64 <= MAX_LIBRARY_BYTES,
            "Asset library exceeds its 64 MB limit"
        );
        std::fs::create_dir_all(&self.root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.root, std::fs::Permissions::from_mode(0o700))?;
        }
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.root.join("assets.lock"))?;
        lock.try_lock()
            .context("The asset library is being saved by another MyPDF window")?;
        let path = self.root.join("assets.json");
        ensure!(!path.is_symlink(), "The library file cannot be a symlink");
        let before = match std::fs::read(&path) {
            Ok(data) => data,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![],
            Err(e) => return Err(e.into()),
        };
        ensure!(
            Sha256::digest(&before).as_slice() == self.fingerprint,
            "The library changed in another process. Reload it before saving."
        );
        let mut tmp = tempfile::NamedTempFile::new_in(&self.root)?;
        tmp.write_all(&raw)?;
        tmp.as_file().sync_all()?;
        tmp.persist(&path)
            .context("Could not save the library; its original file is preserved")?;
        self.fingerprint = Sha256::digest(raw).to_vec();
        Ok(())
    }
}
