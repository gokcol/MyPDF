use super::*;
use anyhow::Context as _;
use mypdf_milestone0::{
    identity::{self, IdentityInfo, PersonalId},
    signing::{self, DesktopSigningRequest, SignatureEvidence},
    write_new,
};
use serde::{Deserialize, Serialize};
use std::io::Read;
use underskrift::CryptoSigner;
use zeroize::Zeroizing;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Panel {
    Identities,
    Sign,
    Validate,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Source {
    File {
        path: PathBuf,
        alias: String,
    },
    Keychain {
        path: Option<PathBuf>,
        fingerprint: String,
    },
    WindowsStore {
        fingerprint: String,
    },
    Certificate,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct DefaultIdentity {
    source: Source,
    sha256: String,
}
#[derive(Default, Serialize, Deserialize)]
struct Catalogue {
    entries: Vec<Entry>,
    #[serde(default)]
    default_identity: Option<DefaultIdentity>,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum CatalogueFile {
    Current(Catalogue),
    Legacy(Vec<Entry>),
}
impl CatalogueFile {
    fn catalogue(self) -> Catalogue {
        match self {
            Self::Current(c) => c,
            Self::Legacy(entries) => Catalogue {
                entries,
                ..Default::default()
            },
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    id: String,
    source: Source,
    info: IdentityInfo,
    der: Vec<u8>,
    eligible: bool,
    explanation: Option<String>,
}
impl Entry {
    fn file(path: PathBuf, imported: identity::ImportedIdentity) -> Self {
        Self {
            id: format!("{:032x}", rand::random::<u128>()),
            source: Source::File {
                path,
                alias: imported.alias,
            },
            info: imported.info,
            der: imported.certificate_der,
            eligible: imported.eligible,
            explanation: imported.explanation,
        }
    }
    fn label(&self) -> String {
        format!(
            "{} • {}",
            self.info.subject,
            match &self.source {
                Source::File { alias, .. } => alias.as_str(),
                Source::Keychain { .. } => "macOS Keychain",
                Source::WindowsStore { .. } => "Windows Personal store",
                Source::Certificate => "public certificate",
            }
        )
    }
    fn signer_name(&self) -> String {
        x509_parser::parse_x509_certificate(&self.der)
            .ok()
            .and_then(|(_, c)| {
                c.subject()
                    .iter_common_name()
                    .next()
                    .and_then(|n| n.as_str().ok())
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| self.info.subject.clone())
    }
    fn reference(&self) -> DefaultIdentity {
        DefaultIdentity {
            source: self.source.clone(),
            sha256: self.info.sha256.clone(),
        }
    }
}
type StoreScan = (Option<PathBuf>, Result<Vec<Entry>, String>);
enum Event {
    Imported(Vec<Entry>),
    Certificate(Entry),
    Created(Entry, PathBuf),
    Exported(PathBuf),
    Verified {
        tab: u64,
        revision: u64,
        report: Vec<SignatureEvidence>,
        explicit_trust: bool,
    },
    Signed {
        path: PathBuf,
        tab: u64,
        revision: u64,
    },
    Preview(Vec<u8>),
    Failed(String),
}
struct ImportDraft {
    path: PathBuf,
    password: Zeroizing<String>,
}
struct ExportDraft {
    entry: Entry,
    password: Zeroizing<String>,
    new_password: Zeroizing<String>,
    confirm: Zeroizing<String>,
}
pub(super) struct DigitalState {
    pub open: bool,
    pub panel: Panel,
    pub busy: bool,
    entries: Vec<Entry>,
    selected: Option<String>,
    default_identity: Option<DefaultIdentity>,
    scanning: bool,
    scan_started: bool,
    was_open: bool,
    store_error: Option<String>,
    store_message: String,
    store_tx: mpsc::Sender<StoreScan>,
    store_rx: mpsc::Receiver<StoreScan>,
    catalogue_error: Option<String>,
    error: Option<String>,
    message: String,
    catalogue: PathBuf,
    catalogue_hash: Vec<u8>,
    import: Option<ImportDraft>,
    export: Option<ExportDraft>,
    creating: bool,
    create_name: String,
    create_org: String,
    create_email: String,
    create_days: u32,
    create_ecdsa: bool,
    create_password: Zeroizing<String>,
    create_confirm: Zeroizing<String>,
    keychain_path: String,
    request: DesktopSigningRequest,
    document: Option<u64>,
    sign_password: Zeroizing<String>,
    prepare: bool,
    visible: bool,
    asset_id: Option<String>,
    preview: Option<TextureHandle>,
    report: Option<(u64, u64, Vec<SignatureEvidence>, bool)>,
    trust_id: Option<String>,
    use_explicit_trust: bool,
    tx: mpsc::Sender<Event>,
    rx: mpsc::Receiver<Event>,
}
fn bounded_read(path: &std::path::Path, limit: u64) -> anyhow::Result<Vec<u8>> {
    let mut bytes = vec![];
    std::fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= limit,
        "The file exceeds the supported size limit"
    );
    Ok(bytes)
}
impl DigitalState {
    pub fn new() -> Self {
        #[cfg(not(test))]
        let root = mypdf_milestone0::assets::data_directory();
        #[cfg(test)]
        let root = std::env::temp_dir().join(format!(
            "mypdf-identities-headless-{:016x}",
            rand::random::<u64>()
        ));
        Self::at(root)
    }
    fn at(root: PathBuf) -> Self {
        use sha2::{Digest, Sha256};
        let catalogue = root.join("identities.json");
        let (contents, catalogue_error, raw) = match bounded_read(&catalogue, 4 * 1024 * 1024) {
            Ok(raw) => match serde_json::from_slice::<CatalogueFile>(&raw)
                .map(CatalogueFile::catalogue)
            {
                Ok(catalogue) if catalogue.entries.len() <= 200 => (catalogue, None, raw),
                _ => (
                    Catalogue::default(),
                    Some(
                        "Could not read the identity catalogue. Its original file is preserved."
                            .into(),
                    ),
                    raw,
                ),
            },
            Err(e) if !catalogue.exists() => {
                let _ = e;
                (Catalogue::default(), None, vec![])
            }
            Err(e) => (Catalogue::default(), Some(format!("{e:#}")), vec![]),
        };
        let Catalogue {
            mut entries,
            default_identity,
        } = contents;
        entries.retain_mut(|entry| {
            let Ok(info) = identity::info(&entry.der) else {
                return false;
            };
            entry.info = info;
            entry.eligible = !matches!(entry.source, Source::Certificate)
                && identity::check_certificate(&entry.der).is_ok();
            if let Source::File { path, .. } = &entry.source
                && !path.is_file()
            {
                entry.eligible = false;
                entry.explanation = Some(
                    "The identity file is unavailable. Import it again if it has moved.".into(),
                );
            }
            if matches!(
                entry.source,
                Source::Keychain { .. } | Source::WindowsStore { .. }
            ) {
                entry.eligible = false;
                entry.explanation = Some("Waiting for certificate-store refresh.".into());
            }
            true
        });
        let (tx, rx) = mpsc::channel();
        let (store_tx, store_rx) = mpsc::channel();
        let mut state = Self {
            open: false,
            panel: Panel::Identities,
            busy: false,
            entries,
            selected: None,
            default_identity,
            scanning: false,
            scan_started: false,
            was_open: false,
            store_error: None,
            store_message: String::new(),
            store_tx,
            store_rx,
            catalogue_error,
            error: None,
            message: String::new(),
            catalogue,
            catalogue_hash: Sha256::digest(raw).to_vec(),
            import: None,
            export: None,
            creating: false,
            create_name: String::new(),
            create_org: String::new(),
            create_email: String::new(),
            create_days: 365,
            create_ecdsa: false,
            create_password: Zeroizing::new(String::new()),
            create_confirm: Zeroizing::new(String::new()),
            keychain_path: String::new(),
            request: DesktopSigningRequest::default(),
            document: None,
            sign_password: Zeroizing::new(String::new()),
            prepare: false,
            visible: true,
            asset_id: None,
            preview: None,
            report: None,
            trust_id: None,
            use_explicit_trust: false,
            tx,
            rx,
        };
        state.select_default();
        state
    }
    pub fn use_document(&mut self, tab: Option<&Tab>) {
        if let Some(tab) = tab
            && self.document != Some(tab.id)
        {
            self.document = Some(tab.id);
            self.select_default();
            self.sign_password = Zeroizing::new(String::new());
            self.request = DesktopSigningRequest::default();
            self.request.page = tab.page;
            if let Some(page) = tab.pages.get(tab.page) {
                let w = 318.0_f32.min(page.width - 20.0);
                let h = 100.0_f32.min(page.height - 20.0);
                self.request.rect = Some([
                    10.0,
                    (page.height - h - 20.0).max(0.0),
                    10.0 + w,
                    (page.height - 20.0).max(h),
                ]);
            }
            self.prepare = tab
                .capabilities
                .as_ref()
                .is_some_and(|c| c.needs_signing_preparation && !c.signed);
            self.preview = None;
        }
        if let Some(entry) = self
            .entries
            .iter()
            .find(|e| Some(&e.id) == self.selected.as_ref())
            && self.request.name.is_empty()
        {
            self.request.name = entry.signer_name();
        }
    }
    fn spawn(
        &mut self,
        ctx: &egui::Context,
        work: impl FnOnce() -> anyhow::Result<Event> + Send + 'static,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        self.message.clear();
        let tx = self.tx.clone();
        let wake = ctx.clone();
        std::thread::spawn(move || {
            let event = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)) {
                Ok(Ok(event)) => event,
                Ok(Err(e)) => Event::Failed(format!("{e:#}")),
                Err(_) => Event::Failed(
                    "The operation could not be completed. Input files are preserved.".into(),
                ),
            };
            let _ = tx.send(event);
            wake.request_repaint();
        });
    }
    fn save_catalogue(&mut self) -> anyhow::Result<()> {
        use sha2::{Digest, Sha256};
        use std::io::Write;
        anyhow::ensure!(
            self.catalogue_error.is_none(),
            "The existing catalogue cannot be overwritten because it could not be read"
        );
        anyhow::ensure!(
            self.entries.len() <= 200,
            "The identity catalogue supports up to 200 entries"
        );
        let parent = self.catalogue.parent().unwrap();
        std::fs::create_dir_all(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
        }
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(parent.join("identities.lock"))?;
        lock.try_lock()?;
        anyhow::ensure!(
            !self.catalogue.is_symlink(),
            "The catalogue cannot be a symlink"
        );
        let before = if self.catalogue.exists() {
            bounded_read(&self.catalogue, 4 * 1024 * 1024)?
        } else {
            vec![]
        };
        anyhow::ensure!(
            Sha256::digest(&before).as_slice() == self.catalogue_hash,
            "The catalogue changed in another process. Restart MyPDF before saving identity references."
        );
        let raw = serde_json::to_vec(&Catalogue {
            entries: self.entries.clone(),
            default_identity: self.default_identity.clone(),
        })?;
        anyhow::ensure!(
            raw.len() <= 4 * 1024 * 1024,
            "The identity catalogue exceeds 4 MB"
        );
        let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
        tmp.write_all(&raw)?;
        tmp.as_file().sync_all()?;
        tmp.persist(&self.catalogue)?;
        self.catalogue_hash = Sha256::digest(raw).to_vec();
        Ok(())
    }
    fn remember(&mut self, mut entry: Entry) {
        let existing = self
            .entries
            .iter()
            .position(|e| e.info.sha256 == entry.info.sha256 && e.source == entry.source);
        if let Some(i) = existing {
            entry.id = self.entries[i].id.clone();
            self.entries[i] = entry.clone();
        } else {
            self.entries.push(entry.clone());
        }
        self.selected = Some(entry.id);
        if let Err(e) = self.save_catalogue() {
            self.error = Some(format!(
                "The identity is available for this session, but its reference could not be saved: {e:#}"
            ));
        }
    }
    fn is_default(&self, entry: &Entry) -> bool {
        self.default_identity.as_ref() == Some(&entry.reference())
    }
    fn select_default(&mut self) {
        if let Some(default) = &self.default_identity {
            self.selected = self
                .entries
                .iter()
                .find(|e| e.eligible && e.reference() == *default)
                .map(|e| e.id.clone());
        }
        if let Some(entry) = self
            .entries
            .iter()
            .find(|e| Some(&e.id) == self.selected.as_ref())
        {
            self.request.name = entry.signer_name();
        }
    }
    fn set_default(&mut self, id: Option<&str>) -> anyhow::Result<()> {
        let reference = id
            .map(|id| {
                self.entries
                    .iter()
                    .find(|e| e.id == id && e.eligible && !matches!(e.source, Source::Certificate))
                    .map(Entry::reference)
                    .context("Choose an available signing identity first")
            })
            .transpose()?;
        let old = std::mem::replace(&mut self.default_identity, reference);
        if let Err(e) = self.save_catalogue() {
            self.default_identity = old;
            return Err(e);
        }
        Ok(())
    }
    fn refresh_store(&mut self, ctx: &egui::Context, path: Option<PathBuf>) {
        if self.scanning {
            return;
        }
        self.scanning = true;
        self.scan_started = true;
        self.store_error = None;
        let tx = self.store_tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let entries = std::panic::catch_unwind(|| discover_store(path.as_deref()))
                .map_err(|_| "Certificate discovery could not complete.".to_string())
                .and_then(|result| result.map_err(|e| format!("{e:#}")));
            let _ = tx.send((path, entries));
            ctx.request_repaint();
        });
    }
    fn reconcile_store(&mut self, path: Option<&std::path::Path>, entries: Vec<Entry>) {
        // Refresh just this store. Missing identities remain visible, but cannot sign.
        for old in &mut self.entries {
            if store_matches(&old.source, path) {
                old.eligible = false;
                old.explanation =
                    Some("This identity is no longer available in the certificate store.".into());
            }
        }
        let count = entries.len();
        for mut entry in entries {
            if let Some(old) = self
                .entries
                .iter_mut()
                .find(|e| e.reference() == entry.reference())
            {
                entry.id = old.id.clone();
                *old = entry;
            } else if self.entries.len() < 200 {
                self.entries.push(entry);
            }
        }
        self.entries.sort_by(|a, b| {
            a.info
                .subject
                .cmp(&b.info.subject)
                .then(a.info.sha256.cmp(&b.info.sha256))
        });
        if self
            .selected
            .as_ref()
            .is_some_and(|id| self.entries.iter().any(|e| &e.id == id && !e.eligible))
        {
            self.selected = None;
            self.sign_password = Zeroizing::new(String::new());
        }
        if self.selected.is_none() {
            self.select_default();
        }
        self.store_message = format!("{count} identities found in {}", store_label());
    }
}
fn store_label() -> &'static str {
    if cfg!(target_os = "windows") {
        "Windows Personal certificate store"
    } else {
        "macOS Keychain"
    }
}
fn store_matches(source: &Source, path: Option<&std::path::Path>) -> bool {
    match source {
        Source::Keychain { path: stored, .. } => {
            cfg!(target_os = "macos") && stored.as_deref() == path
        }
        Source::WindowsStore { .. } => cfg!(target_os = "windows"),
        _ => false,
    }
}
fn discover_store(path: Option<&std::path::Path>) -> anyhow::Result<Vec<Entry>> {
    #[cfg(target_os = "macos")]
    let identities = mypdf_milestone0::keychain::list(path)?;
    #[cfg(target_os = "windows")]
    let identities = {
        let _ = path;
        mypdf_milestone0::windows_store::list()?
    };
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    return Ok(identities
        .into_iter()
        .map(|i| {
            #[cfg(target_os = "macos")]
            let source = Source::Keychain {
                path: path.map(std::path::Path::to_path_buf),
                fingerprint: i.certificate.sha256.clone(),
            };
            #[cfg(target_os = "windows")]
            let source = Source::WindowsStore {
                fingerprint: i.certificate.sha256.clone(),
            };
            Entry {
                id: format!("{:032x}", rand::random::<u128>()),
                source,
                info: i.certificate,
                der: i.certificate_der,
                eligible: i.eligible,
                explanation: (!i.eligible).then(|| {
                    "This certificate does not currently meet the signing requirements.".into()
                }),
            }
        })
        .collect());
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = path;
        anyhow::bail!("Automatic certificate discovery is available on macOS and Windows")
    }
}
enum Action {
    Import,
    Unlock,
    Create,
    CreateSave,
    ReadCertificate,
    Keychain,
    ExportPublic,
    ExportP12,
    ExportSave,
    Preview,
    Place,
    Sign,
    Verify,
    Forget(String),
    SetDefault(String),
    ClearDefault,
}

fn sign_using(
    entry: Entry,
    password: &str,
    bytes: &[u8],
    request: &DesktopSigningRequest,
) -> anyhow::Result<Vec<u8>> {
    let rt = tokio::runtime::Builder::new_current_thread().build()?;
    match entry.source {
        Source::File { path, alias } => {
            let data = bounded_read(&path, 16 * 1024 * 1024)?;
            let signer = identity::FileSigner::load(&data, password, Some(&alias))?;
            anyhow::ensure!(
                identity::fingerprint(signer.certificate_der()) == entry.info.sha256,
                "The identity file has changed. Import it again before signing."
            );
            rt.block_on(signing::sign_custom(bytes, &signer, request))
        }
        Source::Keychain { path, fingerprint } => {
            #[cfg(target_os = "macos")]
            {
                anyhow::ensure!(
                    fingerprint == entry.info.sha256,
                    "Keychain certificate reference has changed. Refresh identities before signing."
                );
                let signer = mypdf_milestone0::keychain::select(path.as_deref(), &fingerprint)?;
                anyhow::ensure!(
                    identity::fingerprint(signer.certificate_der()) == entry.info.sha256,
                    "Keychain certificate does not match the selected identity"
                );
                rt.block_on(signing::sign_custom(bytes, &signer, request))
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = (path, fingerprint);
                anyhow::bail!(
                    "This identity belongs to macOS Keychain. Import a PKCS#12 identity on Windows."
                )
            }
        }
        Source::WindowsStore { fingerprint } => {
            #[cfg(target_os = "windows")]
            {
                anyhow::ensure!(
                    fingerprint == entry.info.sha256,
                    "Certificate reference has changed. Refresh the store before signing."
                );
                let signer = mypdf_milestone0::windows_store::select(&fingerprint)?;
                rt.block_on(signing::sign_custom(bytes, &signer, request))
            }
            #[cfg(not(target_os = "windows"))]
            {
                let _ = fingerprint;
                anyhow::bail!("This identity belongs to the Windows certificate store.")
            }
        }
        Source::Certificate => {
            anyhow::bail!("A public certificate has no private key and cannot sign")
        }
    }
}

impl Viewer {
    pub(super) fn digital_events(&mut self, ctx: &egui::Context) {
        #[cfg(all(not(test), any(target_os = "macos", target_os = "windows")))]
        if !self.digital.scan_started
            || self.digital.open && !self.digital.was_open && !self.digital.scanning
        {
            self.digital.refresh_store(ctx, None);
        }
        if self.digital.open && !self.digital.was_open {
            self.digital.select_default();
        }
        self.digital.was_open = self.digital.open;
        for (path, result) in self.digital.store_rx.try_iter().collect::<Vec<_>>() {
            self.digital.scanning = false;
            match result {
                Ok(entries) => self.digital.reconcile_store(path.as_deref(), entries),
                Err(error) => {
                    self.digital.store_error = Some(format!(
                        "Could not read {}: {error}. Use Refresh to try again.",
                        store_label()
                    ))
                }
            }
            #[cfg(all(not(test), target_os = "macos"))]
            if path.is_none()
                && let Some(DefaultIdentity {
                    source:
                        Source::Keychain {
                            path: Some(path), ..
                        },
                    ..
                }) = &self.digital.default_identity
            {
                self.digital.refresh_store(ctx, Some(path.clone()));
            }
        }
        for event in self.digital.rx.try_iter().collect::<Vec<_>>() {
            self.digital.busy = false;
            match event {
                Event::Imported(entries) => {
                    for entry in entries {
                        self.digital.remember(entry);
                    }
                    self.digital.import = None;
                    self.digital.message = "Identity references imported. Passphrases and private keys are not stored in settings.".into();
                }
                Event::Certificate(entry) => {
                    self.digital.remember(entry);
                    self.digital.message =
                        "Public certificate imported for inspection and explicit verification."
                            .into();
                }
                Event::Created(entry, path) => {
                    self.digital.remember(entry);
                    self.digital.creating = false;
                    self.digital.message = format!(
                        "Personal ID saved to {}. Its self-signed certificate needs separate recipient trust.",
                        path.display()
                    );
                }
                Event::Exported(path) => {
                    self.digital.message = format!(
                        "Exported {}. Existing files were preserved.",
                        path.display()
                    )
                }
                Event::Preview(png) => match mypdf_milestone0::assets::decode_png(&png) {
                    Ok(image) => {
                        let image = image.into_rgba8();
                        self.digital.preview = Some(ctx.load_texture(
                            "signature_preview",
                            egui::ColorImage::from_rgba_unmultiplied(
                                [image.width() as usize, image.height() as usize],
                                &image,
                            ),
                            egui::TextureOptions::LINEAR,
                        ));
                    }
                    Err(e) => self.digital.error = Some(format!("{e:#}")),
                },
                Event::Verified {
                    tab,
                    revision,
                    report,
                    explicit_trust,
                } => {
                    self.digital.report = Some((tab, revision, report, explicit_trust));
                    self.digital.panel = Panel::Validate;
                }
                Event::Signed {
                    path,
                    tab,
                    revision,
                } => {
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab)
                        && let Some(s) = &mut tab.session
                    {
                        s.mark_saved(revision);
                    }
                    self.open(path.clone(), ctx);
                    self.digital.document = None;
                    self.digital.report = None;
                    self.digital.panel = Panel::Validate;
                    self.digital.message = format!(
                        "Signed copy saved to {} and independently reopened by the viewer. Earlier signed bytes were preserved.",
                        path.display()
                    );
                }
                Event::Failed(error) => self.digital.error = Some(error),
            }
        }
        if let Some((tab, page, rect)) = self.editor.pending_sign_rect.take() {
            self.digital.document = Some(tab);
            self.digital.request.page = page;
            self.digital.request.rect = Some(rect);
            self.digital.visible = true;
            self.digital.preview = None;
            self.digital.open = true;
            self.digital.panel = Panel::Sign;
        }
    }
    pub(super) fn digital_window(&mut self, ctx: &egui::Context) {
        if !self.digital.open {
            return;
        }
        let before = (
            self.digital.request.clone(),
            self.digital.asset_id.clone(),
            self.digital.visible,
        );
        let active = self.tabs.get(self.active);
        let mut open = true;
        let mut action = None;
        let assets = self
            .editor
            .library
            .as_ref()
            .map(|l| l.entries.as_slice())
            .unwrap_or(&[]);
        egui::Window::new("Digital IDs and signatures").open(&mut open).collapsible(false).vscroll(true).max_height((ctx.content_rect().height()-160.0).max(260.0)).default_width(610.0).default_height(540.0).show(ctx, |ui| {
            ui.horizontal(|ui| { for (p, name) in [(Panel::Identities, "Digital IDs"), (Panel::Sign, "Sign document"), (Panel::Validate, "Validate")] { ui.selectable_value(&mut self.digital.panel, p, name); } });
 ui.separator();
            if self.digital.busy { ui.horizontal(|ui| { ui.spinner();
 ui.label("Working… Input files are preserved."); }); }
            if let Some(error) = &self.digital.error { ui.colored_label(Color32::from_rgb(155, 47, 39), error); }
            if let Some(error) = &self.digital.catalogue_error { ui.colored_label(Color32::from_rgb(155, 47, 39), error); }
            if !self.digital.message.is_empty() { ui.label(RichText::new(&self.digital.message).color(ACCENT)); }
            if self.digital.scanning { ui.horizontal(|ui| { ui.spinner(); ui.label(format!("Loading {}…", store_label())); }); }
            if let Some(error) = &self.digital.store_error { ui.colored_label(Color32::from_rgb(155, 47, 39), error); }
            else if !self.digital.store_message.is_empty() { ui.label(RichText::new(&self.digital.store_message).small().color(MUTED)); }
            if self.digital.default_identity.is_some() && !self.digital.entries.iter().any(|e| e.eligible && self.digital.is_default(e)) {
                ui.label(RichText::new("Your default certificate is unavailable. Refresh the store or choose another identity.").small().color(MUTED));
                if ui.small_button("Clear default certificate").clicked() { action = Some(Action::ClearDefault); }
            }
            ui.add_enabled_ui(!self.digital.busy, |ui| {
                match self.digital.panel {
                    Panel::Identities => {
                        ui.horizontal_wrapped(|ui| { if ui.add(ActionButton::new(Icon::Import, "Import .p12 / .pfx…")).clicked() { action = Some(Action::Import); }
 if ui.add(ActionButton::new(Icon::Shield, "Create personal ID…")).clicked() { action = Some(Action::Create); }
 if ui.add(ActionButton::new(Icon::Open, "Read .cer / .crt / .pem…")).clicked() { action = Some(Action::ReadCertificate); } });
                        #[cfg(any(target_os = "macos", target_os = "windows"))]
                        { if ui.add_enabled(!self.digital.scanning, ActionButton::new(Icon::Reload, "Refresh certificates")).clicked() { action = Some(Action::Keychain); } }
                        #[cfg(target_os = "macos")]
                        egui::CollapsingHeader::new("Other Keychain").show(ui, |ui| {
                            let r = ui.add(egui::TextEdit::singleline(&mut self.digital.keychain_path).hint_text("Optional keychain file; empty uses system search list")); self.editor.text_editing |= r.has_focus();
                        });
                        ui.separator();
                        for entry in &self.digital.entries { ui.horizontal(|ui| { let label = if self.digital.default_identity.as_ref() == Some(&entry.reference()) { format!("{} · Default", entry.label()) } else { entry.label() };
 let selected = self.digital.selected.as_ref() == Some(&entry.id);
 if ui.add_sized(vec2((ui.available_width()-138.0).max(120.0), 28.0), egui::Button::new(&label).selected(selected).truncate()).on_hover_text(label).clicked() { self.digital.selected = Some(entry.id.clone()); }
 if !matches!(entry.source, Source::Keychain { .. } | Source::WindowsStore { .. }) && ui.small_button("Forget reference").clicked() { action = Some(Action::Forget(entry.id.clone())); } }); }
                        if self.digital.entries.is_empty() { ui.label("Import a protected identity, create a personal ID, or select one from Keychain."); }
                        if let Some(entry) = self.digital.entries.iter().find(|e| Some(&e.id) == self.digital.selected.as_ref()) { ui.separator(); certificate_details(ui, entry);
 if let Some(choice) = default_choice(ui, &self.digital, entry) { action = Some(choice); }
 ui.horizontal(|ui| { if ui.add(ActionButton::new(Icon::Save, "Export public certificate…")).clicked() { action = Some(Action::ExportPublic); }
 if ui.add_enabled(matches!(entry.source, Source::File { .. }), egui::Button::new("Export encrypted .p12…")).clicked() { action = Some(Action::ExportP12); } }); }
                        if let Some(import) = &mut self.digital.import { ui.separator();
 ui.label(format!("Unlock {}", import.path.file_name().unwrap_or_default().to_string_lossy()));
 let r = ui.add(egui::TextEdit::singleline(&mut *import.password).password(true).hint_text("Identity passphrase").char_limit(1024)); self.editor.text_editing |= r.has_focus();
 if ui.button("Unlock and import").clicked() { action = Some(Action::Unlock); }
 if ui.button("Cancel import").clicked() { self.digital.import = None; } }
                        if self.digital.creating { ui.separator();
 ui.heading("Create personal digital ID");
 let r = ui.add(egui::TextEdit::singleline(&mut self.digital.create_name).hint_text("Full name").char_limit(200)); self.editor.text_editing |= r.has_focus();
 ui.add(egui::TextEdit::singleline(&mut self.digital.create_org).hint_text("Organization (optional)").char_limit(200));
 ui.add(egui::TextEdit::singleline(&mut self.digital.create_email).hint_text("Email (optional)").char_limit(254));
                            ui.horizontal(|ui| { ui.label("Valid for days");
 ui.add(egui::DragValue::new(&mut self.digital.create_days).range(1..=3650));
 ui.selectable_value(&mut self.digital.create_ecdsa, false, "RSA 2048");
 ui.selectable_value(&mut self.digital.create_ecdsa, true, "P-256"); });
                            ui.add(egui::TextEdit::singleline(&mut *self.digital.create_password).password(true).hint_text("New passphrase (at least 12 bytes)").char_limit(1024));
 ui.add(egui::TextEdit::singleline(&mut *self.digital.create_confirm).password(true).hint_text("Confirm passphrase").char_limit(1024));
 ui.label(RichText::new("A self-signed ID signs cryptographically; it does not become trusted by Adobe or change trusted roots.").small().color(MUTED));
                            if ui.button("Create and save .p12…").clicked() { action = Some(Action::CreateSave); }
 if ui.button("Cancel creation").clicked() { self.digital.creating = false; self.digital.create_password = Zeroizing::new(String::new()); self.digital.create_confirm = Zeroizing::new(String::new()); }
                        }
                        if let Some(export) = &mut self.digital.export { ui.separator();
 ui.heading("Export encrypted identity backup");
 ui.add(egui::TextEdit::singleline(&mut *export.password).password(true).hint_text("Current identity passphrase").char_limit(1024));
 ui.add(egui::TextEdit::singleline(&mut *export.new_password).password(true).hint_text("New backup passphrase (at least 12 bytes)").char_limit(1024));
 ui.add(egui::TextEdit::singleline(&mut *export.confirm).password(true).hint_text("Confirm new passphrase").char_limit(1024));
 if ui.button("Export backup…").clicked() { action = Some(Action::ExportSave); }
 if ui.button("Cancel export").clicked() { self.digital.export = None; } }
                    }
                    Panel::Sign => {
                        if let Some(tab) = active { ui.label(RichText::new(tab.name()).strong());
 if self.digital.document != Some(tab.id) && ui.button("Use active document").clicked() { self.digital.use_document(Some(tab)); } }
                        else { ui.label("Open a PDF to sign it."); }
                        #[cfg(any(target_os = "macos", target_os = "windows"))]
                        if ui.add_enabled(!self.digital.scanning, ActionButton::new(Icon::Reload, "Refresh certificates")).clicked() { action = Some(Action::Keychain); }
                        egui::ComboBox::from_id_salt("signing_id").selected_text(self.digital.entries.iter().find(|e| Some(&e.id) == self.digital.selected.as_ref()).map_or("Choose signing identity".into(), Entry::label)).width(480.0).show_ui(ui, |ui| { for entry in self.digital.entries.iter().filter(|e| e.eligible && !matches!(e.source, Source::Certificate)) { if ui.selectable_value(&mut self.digital.selected, Some(entry.id.clone()), entry.label()).clicked() { self.digital.request.name = entry.signer_name(); self.digital.sign_password = Zeroizing::new(String::new()); self.digital.preview = None;
 ui.close(); } } });
                        if let Some(entry) = self.digital.entries.iter().find(|e| Some(&e.id) == self.digital.selected.as_ref()) { if let Some(choice) = default_choice(ui, &self.digital, entry) { action = Some(choice); }
 egui::CollapsingHeader::new("Certificate details").show(ui, |ui| certificate_details(ui, entry));
 if matches!(entry.source, Source::File { .. }) { ui.add(egui::TextEdit::singleline(&mut *self.digital.sign_password).password(true).hint_text("Identity passphrase").char_limit(1024)); } }
                        ui.add(egui::TextEdit::singleline(&mut self.digital.request.field).hint_text("New signature field name").char_limit(100));
 ui.add(egui::TextEdit::singleline(&mut self.digital.request.name).hint_text("Appearance name").char_limit(200));
 ui.add(egui::TextEdit::singleline(&mut self.digital.request.reason).hint_text("Reason (optional)").char_limit(1000));
 ui.add(egui::TextEdit::singleline(&mut self.digital.request.location).hint_text("Location (optional)").char_limit(300));
                        ui.checkbox(&mut self.digital.visible, "Visible signature appearance");
                        if self.digital.visible { if let Some(tab) = active { ui.horizontal(|ui| { let mut page = self.digital.request.page + 1;
 ui.label("Page");
 if ui.add(egui::DragValue::new(&mut page).range(1..=tab.pages.len().max(1))).changed() { self.digital.request.page = page - 1; self.digital.preview = None; }
 if ui.button("Draw signature rectangle on page").clicked() { action = Some(Action::Place); } }); }
                            if let Some(r) = &mut self.digital.request.rect { egui::Grid::new("signature_rect").num_columns(4).show(ui, |ui| { let mut x = r[0];
 let mut y = r[1];
 let mut w = r[2] - r[0];
 let mut h = r[3] - r[1]; for (name, value) in [("Left", &mut x), ("Top", &mut y), ("Width", &mut w), ("Height", &mut h)] { ui.label(name);
 ui.add(egui::DragValue::new(value).range(0.0..=1500.0).speed(1.0)); } *r = [x, y, x + w, y + h]; }); }
                            egui::ComboBox::from_id_salt("signature_artwork").selected_text(self.digital.asset_id.as_ref().and_then(|id| assets.iter().find(|a| &a.id == id)).map_or("No artwork", |a| a.name.as_str())).show_ui(ui, |ui| { if ui.selectable_value(&mut self.digital.asset_id, None, "No artwork").clicked() { ui.close(); } for asset in assets { if ui.selectable_value(&mut self.digital.asset_id, Some(asset.id.clone()), &asset.name).clicked() { ui.close(); } } });
                            if ui.add(ActionButton::new(Icon::Read, "Preview appearance")).clicked() { action = Some(Action::Preview); }
 if let Some(image) = &self.digital.preview { ui.add(egui::Image::new(image).fit_to_exact_size(vec2(480.0_f32.min(image.size_vec2().x / 2.0), (image.size_vec2().y / image.size_vec2().x * 480.0).min(220.0)))); }
                        }
                        if let Some(tab) = active {
                            if let Some(caps) = &tab.capabilities { if caps.certified { ui.colored_label(Color32::from_rgb(155, 47, 39), "Certification restrictions are unsupported; this PDF cannot be extended here."); }
 if caps.needs_signing_preparation { if caps.signed { ui.colored_label(Color32::from_rgb(155, 47, 39), "This signed PDF uses xref streams. It cannot be rewritten for this writer."); } else { ui.checkbox(&mut self.digital.prepare, "Prepare an unsigned signing copy (required for this PDF)"); } } }
                            if tab.session.as_ref().is_some_and(|s| s.revision() != 0) { ui.label("Current annotations will be included in the signed copy."); }
                        }
                        ui.label(RichText::new("New approval signature • offline verification • local signing time only. Save to a new filename.").small().color(MUTED));
                        let usable = active.is_some_and(|t| t.bytes.is_some() && self.digital.document == Some(t.id) && t.capabilities.as_ref().is_some_and(|c| !(c.certified || c.encrypted || c.signed && c.needs_signing_preparation))) && self.digital.entries.iter().any(|e| Some(&e.id) == self.digital.selected.as_ref() && e.eligible && !matches!(e.source, Source::Certificate));
                        if ui.add_enabled(usable, ActionButton::new(Icon::Sign, "Sign and save a copy…").primary()).clicked() { action = Some(Action::Sign); }
                    }
                    Panel::Validate => {
                        if let Some(tab) = active { ui.label(RichText::new(tab.name()).strong()); }
                        ui.checkbox(&mut self.digital.use_explicit_trust, "Verify with an explicitly selected certificate for this session");
                        if self.digital.use_explicit_trust { egui::ComboBox::from_id_salt("verify_cert").selected_text(self.digital.entries.iter().find(|e| Some(&e.id) == self.digital.trust_id.as_ref()).map_or("Choose public certificate".into(), Entry::label)).show_ui(ui, |ui| { for entry in &self.digital.entries { if ui.selectable_value(&mut self.digital.trust_id, Some(entry.id.clone()), entry.label()).clicked() { ui.close(); } } }); }
                        if ui.add_enabled(active.is_some_and(|t| t.bytes.is_some()), egui::Button::new("Validate signatures")).clicked() { action = Some(Action::Verify); }
                        ui.label(RichText::new("Integrity, certificate trust and later revisions are separate checks. Revocation and timestamp trust are not checked online. Trusted roots are never modified.").small().color(MUTED));
                        if let Some((id, revision, report, trust)) = &self.digital.report {
                            if active.is_none_or(|t| t.id != *id || t.session.as_ref().map_or(0, |s| s.revision()) != *revision) { ui.colored_label(Color32::from_rgb(155, 47, 39), "This report belongs to another document/revision. Validate the active document again."); }
                            else { if report.is_empty() { ui.label("No applied digital signatures were found."); } for sig in report { ui.separator();
 ui.heading(&sig.field); if let Some(info) = &sig.signer { ui.label(format!("Signer: {}", info.subject)); ui.collapsing("Signing certificate", |ui| certificate_info(ui, info)); } check_row(ui, "Signature mathematics", sig.crypto_valid); check_row(ui, "Signed content digest", sig.digest_matches); check_row(ui, "Signature byte range", sig.byte_range_valid); check_row(ui, "Certificate binding", sig.certificate_binding_valid);
 ui.label(format!("Identity trust: {}", if sig.trusted && *trust { "trusted against the selected certificate for this session" } else { "unknown / untrusted" }));
 ui.label(format!("Coverage: {}", if sig.covers_current_file { "current file" } else { "signed revision; later bytes exist" }));
 ui.label(format!("Claimed signing time: {}", sig.claimed_signing_time.as_deref().unwrap_or("not provided")));
 ui.label(format!("Certificate validity: {}", sig.certificate_validity));
 ui.label("Revocation: not checked (offline)");
 if let Some(t) = &sig.timestamp { ui.label(format!("Reported timestamp: {t} (trust not independently evaluated here)")); } } }
                        }
                    }
                }
            });
        });
        if before
            != (
                self.digital.request.clone(),
                self.digital.asset_id.clone(),
                self.digital.visible,
            )
        {
            self.digital.preview = None;
        }
        if !open {
            self.digital.sign_password = Zeroizing::new(String::new());
            self.digital.create_password = Zeroizing::new(String::new());
            self.digital.create_confirm = Zeroizing::new(String::new());
            self.digital.import = None;
            self.digital.export = None;
        }
        self.digital.open = open;
        if let Some(action) = action {
            self.digital_action(action, ctx);
        }
    }
    fn digital_action(&mut self, action: Action, ctx: &egui::Context) {
        if self.digital.busy {
            return;
        }
        self.digital.error = None;
        match action {
            Action::Import => {
                if let Some(path) = self.native_pick(
                    rfd::FileDialog::new()
                        .set_title("Import signing identity")
                        .add_filter("PKCS#12 identity", &["p12", "pfx"]),
                ) {
                    self.digital.import = Some(ImportDraft {
                        path,
                        password: Zeroizing::new(String::new()),
                    });
                }
            }
            Action::Unlock => {
                if let Some(import) = &mut self.digital.import {
                    let path = import.path.clone();
                    let password = std::mem::take(&mut import.password);
                    self.digital.spawn(ctx, move || {
                        let data = bounded_read(&path, 16 * 1024 * 1024)?;
                        Ok(Event::Imported(
                            identity::inspect_pkcs12(&data, &password)?
                                .into_iter()
                                .map(|i| Entry::file(path.clone(), i))
                                .collect(),
                        ))
                    });
                }
            }
            Action::Create => self.digital.creating = true,
            Action::CreateSave => {
                if self.digital.create_password.len() < 12
                    || *self.digital.create_password != *self.digital.create_confirm
                {
                    self.digital.error =
                        Some("Use matching passphrases of at least 12 bytes.".into());
                    return;
                }
                if let Some(path) = self.native_save(
                    rfd::FileDialog::new()
                        .set_title("Save personal digital ID • new filename")
                        .add_filter("PKCS#12 identity", &["p12"])
                        .set_file_name("personal-id.p12"),
                ) {
                    let details = PersonalId {
                        name: self.digital.create_name.clone(),
                        organization: self.digital.create_org.clone(),
                        email: self.digital.create_email.clone(),
                        validity_days: self.digital.create_days,
                        ecdsa: self.digital.create_ecdsa,
                    };
                    let password = std::mem::take(&mut self.digital.create_password);
                    self.digital.create_confirm = Zeroizing::new(String::new());
                    self.digital.spawn(ctx, move || {
                        let created = identity::create_personal(&details, &password)?;
                        write_new(&path, &created.pkcs12)?;
                        let imported = identity::inspect_pkcs12(&created.pkcs12, &password)?
                            .into_iter()
                            .next()
                            .context("Created identity was not readable")?;
                        Ok(Event::Created(Entry::file(path.clone(), imported), path))
                    });
                }
            }
            Action::ReadCertificate => {
                if let Some(path) = self.native_pick(
                    rfd::FileDialog::new()
                        .set_title("Inspect public certificate")
                        .add_filter("X.509 certificate", &["cer", "crt", "pem"]),
                ) {
                    self.digital.spawn(ctx, move || { let der = identity::read_certificate(&bounded_read(&path, 1024 * 1024)?)?;
 let info = identity::info(&der)?; Ok(Event::Certificate(Entry { id: format!("{:032x}", rand::random::<u128>()), source: Source::Certificate, info, der, eligible: false, explanation: Some("A public certificate has no private key. It can be inspected/exported and used for explicit verification, but cannot sign.".into()) })) });
                }
            }
            Action::Keychain => {
                let path = (!self.digital.keychain_path.trim().is_empty())
                    .then(|| PathBuf::from(self.digital.keychain_path.trim()));
                self.digital.refresh_store(ctx, path);
            }
            Action::ExportPublic => {
                if let Some(entry) = self
                    .digital
                    .entries
                    .iter()
                    .find(|e| Some(&e.id) == self.digital.selected.as_ref())
                    .cloned()
                    && let Some(path) = self.native_save(
                        rfd::FileDialog::new()
                            .set_title("Export public certificate • new filename")
                            .add_filter("X.509 certificate", &["cer"])
                            .set_file_name("certificate.cer"),
                    )
                {
                    self.digital.spawn(ctx, move || {
                        write_new(&path, &entry.der)?;
                        Ok(Event::Exported(path))
                    });
                }
            }
            Action::ExportP12 => {
                if let Some(entry) = self
                    .digital
                    .entries
                    .iter()
                    .find(|e| Some(&e.id) == self.digital.selected.as_ref())
                    .cloned()
                {
                    self.digital.export = Some(ExportDraft {
                        entry,
                        password: Zeroizing::new(String::new()),
                        new_password: Zeroizing::new(String::new()),
                        confirm: Zeroizing::new(String::new()),
                    });
                }
            }
            Action::ExportSave => {
                if let Some(export) = &mut self.digital.export {
                    if export.new_password.len() < 12 || *export.new_password != *export.confirm {
                        self.digital.error =
                            Some("Use matching backup passphrases of at least 12 bytes.".into());
                        return;
                    }
                    if let Some(destination) =
                        menu::with_file_dialog(self._native_menu.as_ref(), || {
                            rfd::FileDialog::new()
                                .set_title("Export encrypted identity • new filename")
                                .add_filter("PKCS#12 identity", &["p12", "pfx"])
                                .set_file_name("identity-backup.p12")
                                .save_file()
                        })
                    {
                        let entry = export.entry.clone();
                        let password = std::mem::take(&mut export.password);
                        let new_password = std::mem::take(&mut export.new_password);
                        export.confirm = Zeroizing::new(String::new());
                        self.digital.spawn(ctx, move || { if let Source::File { path, alias } = entry.source { let signer = identity::FileSigner::load(&bounded_read(&path, 16 * 1024 * 1024)?, &password, Some(&alias))?; anyhow::ensure!(identity::fingerprint(signer.certificate_der()) == entry.info.sha256, "The identity file has changed. Import it again.");
 let data = signer.export(&alias, &new_password)?; write_new(&destination, &data)?; Ok(Event::Exported(destination)) } else { anyhow::bail!("Keychain private keys are not exported. Export its public certificate instead.") } });
                    }
                }
            }
            Action::Preview => {
                let mut request = self.digital.request.clone();
                request.artwork = self.digital.asset_id.as_ref().and_then(|id| {
                    self.editor
                        .library
                        .as_ref()?
                        .entries
                        .iter()
                        .find(|a| &a.id == id)
                        .cloned()
                });
                self.digital.spawn(ctx, move || {
                    Ok(Event::Preview(signing::signature_artwork(&request)?))
                });
            }
            Action::Place => {
                self.digital.open = false;
                self.editor.mode = editing::Mode::Sign;
                self.editor.start_signature_rectangle();
                self.status = Some((
                    "Drag a visible signature rectangle on the page, at least 120 × 48 points."
                        .into(),
                    false,
                ));
            }
            Action::Sign => {
                if !self.apply_pending_annotation() {
                    return;
                }
                let Some(tab) = self.tabs.get(self.active) else {
                    return;
                };
                let Some(bytes) = tab
                    .session
                    .as_ref()
                    .map(|s| s.source.clone())
                    .or_else(|| tab.bytes.clone())
                else {
                    return;
                };
                let Some(entry) = self
                    .digital
                    .entries
                    .iter()
                    .find(|e| Some(&e.id) == self.digital.selected.as_ref())
                    .cloned()
                else {
                    return;
                };
                if self.digital.document != Some(tab.id) {
                    self.digital.error =
                        Some("Choose the active document again before signing.".into());
                    return;
                }
                let tab_id = tab.id;
                let revision = tab.session.as_ref().map_or(0, |s| s.revision());
                let annotations = tab
                    .session
                    .as_ref()
                    .map(|s| s.annotations.clone())
                    .unwrap_or_default();
                let prepare = self.digital.prepare;
                let mut request = self.digital.request.clone();
                if !self.digital.visible {
                    request.rect = None;
                }
                request.artwork = self.digital.asset_id.as_ref().and_then(|id| {
                    self.editor
                        .library
                        .as_ref()?
                        .entries
                        .iter()
                        .find(|a| &a.id == id)
                        .cloned()
                });
                if let Some(path) = self.native_save(
                    rfd::FileDialog::new()
                        .set_title("Save signed PDF • new filename")
                        .add_filter("PDF document", &["pdf"])
                        .set_file_name(format!(
                            "{}-signed.pdf",
                            tab.path.file_stem().unwrap_or_default().to_string_lossy()
                        ))
                        .set_directory(tab.path.parent().unwrap_or(std::path::Path::new("."))),
                ) {
                    let password = std::mem::take(&mut self.digital.sign_password);
                    self.digital.spawn(ctx, move || {
                        let mut input = if revision == 0 {
                            bytes.to_vec()
                        } else {
                            mypdf_milestone0::annotations::export(&bytes, &annotations)?
                        };
                        if prepare {
                            input = mypdf_milestone0::document::prepare_signing_copy(&input)?;
                        }
                        let signed = sign_using(entry, &password, &input, &request)?;
                        write_new(&path, &signed)?;
                        Ok(Event::Signed {
                            path,
                            tab: tab_id,
                            revision,
                        })
                    });
                }
            }
            Action::Verify => {
                if let Some(tab) = self.tabs.get(self.active)
                    && let Some(bytes) = tab.bytes.clone()
                {
                    let tab_id = tab.id;
                    let revision = tab.session.as_ref().map_or(0, |s| s.revision());
                    let explicit_trust = self.digital.use_explicit_trust;
                    let trusted = if explicit_trust {
                        let Some(entry) = self
                            .digital
                            .entries
                            .iter()
                            .find(|e| Some(&e.id) == self.digital.trust_id.as_ref())
                        else {
                            self.digital.error =
                                Some("Select a certificate for explicit verification.".into());
                            return;
                        };
                        Some(entry.der.clone())
                    } else {
                        None
                    };
                    self.digital.spawn(ctx, move || {
                        Ok(Event::Verified {
                            tab: tab_id,
                            revision,
                            report: signing::verify(&bytes, trusted.as_deref())?,
                            explicit_trust,
                        })
                    });
                }
            }
            Action::SetDefault(id) => match self.digital.set_default(Some(&id)) {
                Ok(()) => self.digital.message = "Default signing certificate saved.".into(),
                Err(e) => self.digital.error = Some(format!("{e:#}")),
            },
            Action::ClearDefault => {
                if let Err(e) = self.digital.set_default(None) {
                    self.digital.error = Some(format!("{e:#}"));
                } else {
                    self.digital.message = "Default certificate cleared.".into();
                }
            }
            Action::Forget(id) => {
                if self
                    .digital
                    .entries
                    .iter()
                    .any(|e| e.id == id && self.digital.is_default(e))
                {
                    self.digital.default_identity = None;
                }
                self.digital.entries.retain(|e| e.id != id);
                if self.digital.selected.as_ref() == Some(&id) {
                    self.digital.selected = None;
                }
                if let Err(e) = self.digital.save_catalogue() {
                    self.digital.error = Some(format!("{e:#}"));
                }
            }
        }
    }
}

fn default_choice(ui: &mut egui::Ui, state: &DigitalState, entry: &Entry) -> Option<Action> {
    if state.is_default(entry) {
        let mut clear = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new("✓ Default signing certificate").color(ACCENT));
            clear = ui.small_button("Clear default").clicked();
        });
        clear.then_some(Action::ClearDefault)
    } else if ui
        .add_enabled(
            entry.eligible && !matches!(entry.source, Source::Certificate),
            ActionButton::new(Icon::Star, "Use as default certificate"),
        )
        .clicked()
    {
        Some(Action::SetDefault(entry.id.clone()))
    } else {
        None
    }
}

fn certificate_details(ui: &mut egui::Ui, entry: &Entry) {
    certificate_info(ui, &entry.info);
    if let Some(explanation) = &entry.explanation {
        ui.label(RichText::new(explanation).small().color(MUTED));
    }
}
fn certificate_info(ui: &mut egui::Ui, info: &IdentityInfo) {
    egui::Grid::new((&info.sha256, "certificate"))
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            for (label, value) in [
                ("Subject", &info.subject),
                ("Issuer", &info.issuer),
                ("Valid from", &info.not_before),
                ("Valid until", &info.not_after),
                ("Algorithm", &info.algorithm),
                ("SHA-256", &info.sha256),
            ] {
                ui.label(label);
                ui.add(egui::Label::new(value).wrap());
                ui.end_row();
            }
        });
    if info.self_issued {
        ui.label(
            RichText::new("Self-issued identity; trust is a separate recipient decision.")
                .small()
                .color(MUTED),
        );
    }
}
fn check_row(ui: &mut egui::Ui, label: &str, valid: bool) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.colored_label(
            if valid {
                ACCENT
            } else {
                Color32::from_rgb(155, 47, 39)
            },
            if valid { "Passed" } else { "FAILED" },
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_certificate_persists_migrates_and_never_falls_back() {
        let temp = tempfile::tempdir().unwrap();
        let created =
            identity::create("Default Test", "synthetic-default-passphrase", true).unwrap();
        let file = temp.path().join("identity.p12");
        std::fs::write(&file, &created.pkcs12).unwrap();
        let imported = identity::inspect_pkcs12(&created.pkcs12, "synthetic-default-passphrase")
            .unwrap()
            .remove(0);
        let entry = Entry::file(file.clone(), imported);
        // Read the old catalogue format and migrate without changing identity IDs.
        std::fs::write(
            temp.path().join("identities.json"),
            serde_json::to_vec(&vec![entry.clone()]).unwrap(),
        )
        .unwrap();
        let mut state = DigitalState::at(temp.path().into());
        assert!(state.default_identity.is_none());
        state.set_default(Some(&entry.id)).unwrap();
        let mut state = DigitalState::at(temp.path().into());
        assert_eq!(state.selected, Some(entry.id.clone()));
        assert_eq!(state.request.name, "Default Test");
        // Refresh/import of the same identity preserves the reference and preference.
        let mut again = entry.clone();
        again.id = "new-import-id".into();
        state.remember(again);
        assert_eq!(state.entries.len(), 1);
        assert!(state.is_default(&state.entries[0]));
        assert_eq!(state.selected, Some(entry.id.clone()));
        // A missing default is not substituted with an arbitrary signing identity.
        std::fs::remove_file(file).unwrap();
        let state = DigitalState::at(temp.path().into());
        assert!(state.selected.is_none());
        assert!(state.default_identity.is_some());
        assert!(!state.entries[0].eligible);
    }
    #[test]
    fn store_refresh_preserves_default_and_disables_removed_identities() {
        let temp = tempfile::tempdir().unwrap();
        let created =
            identity::create("Store Default Test", "synthetic-store-passphrase", true).unwrap();
        let info = identity::info(&created.certificate_der).unwrap();
        let source = if cfg!(target_os = "windows") {
            Source::WindowsStore {
                fingerprint: info.sha256.clone(),
            }
        } else {
            Source::Keychain {
                path: None,
                fingerprint: info.sha256.clone(),
            }
        };
        let entry = Entry {
            id: "store-one".into(),
            source,
            info,
            der: created.certificate_der,
            eligible: true,
            explanation: None,
        };
        let mut state = DigitalState::at(temp.path().into());
        state.reconcile_store(None, vec![entry.clone()]);
        assert!(
            state.selected.is_none(),
            "discovery never silently chooses a certificate"
        );
        state.set_default(Some(&entry.id)).unwrap();
        let mut state = DigitalState::at(temp.path().into());
        assert!(
            state.selected.is_none(),
            "cached store references must be refreshed first"
        );
        let mut fresh = entry.clone();
        fresh.id = "different-discovery-id".into();
        state.reconcile_store(None, vec![fresh.clone()]);
        assert_eq!(state.selected.as_deref(), Some("store-one"));
        assert_eq!(state.entries.len(), 1);
        state.reconcile_store(None, vec![]);
        assert!(state.selected.is_none());
        assert!(state.default_identity.is_some());
        assert!(state.set_default(Some("store-one")).is_err());
        state.reconcile_store(None, vec![fresh]);
        assert_eq!(state.selected.as_deref(), Some("store-one"));
        state.set_default(None).unwrap();
        assert!(
            DigitalState::at(temp.path().into())
                .default_identity
                .is_none()
        );
        let mut public = entry;
        public.id = "public-only".into();
        public.source = Source::Certificate;
        public.eligible = false;
        state.remember(public);
        assert!(state.set_default(Some("public-only")).is_err());
    }
    #[test]
    fn discovery_completion_does_not_finish_an_unrelated_signing_operation() {
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.digital.busy = true;
        app.digital.scanning = true;
        app.digital
            .store_tx
            .send((None, Err("Store locked".into())))
            .unwrap();
        app.digital_events(&ctx);
        assert!(app.digital.busy);
        assert!(!app.digital.scanning);
        assert!(
            app.digital
                .store_error
                .as_deref()
                .unwrap()
                .contains("Store locked")
        );
    }
    #[test]
    fn catalogue_contains_only_public_metadata_and_detects_other_writers() {
        let id = identity::create("Catalogue Test", "synthetic-test-password-only", true).unwrap();
        let imported = identity::inspect_pkcs12(&id.pkcs12, "synthetic-test-password-only")
            .unwrap()
            .remove(0);
        let mut state = DigitalState::new();
        let temp = tempfile::tempdir().unwrap();
        state.catalogue = temp.path().join("identities.json");
        state.remember(Entry::file(temp.path().join("identity.p12"), imported));
        assert!(state.error.is_none());
        let bytes = std::fs::read(&state.catalogue).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("identity.p12"));
        assert!(!text.contains("synthetic-test-password-only"));
        assert!(!text.contains("PRIVATE KEY"));
        assert!(identity::inspect_pkcs12(text.as_bytes(), "synthetic-test-password-only").is_err());
        std::fs::write(&state.catalogue, b"[]").unwrap();
        assert!(state.save_catalogue().is_err());
        assert_eq!(std::fs::read(&state.catalogue).unwrap(), b"[]");
    }
}
