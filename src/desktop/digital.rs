use super::*;
use anyhow::Context as _;
use mypdf_milestone0::{
    identity::{self, IdentityInfo, PersonalId},
    signing::{self, AppearanceLayout, DesktopSigningRequest, SignatureEvidence},
    write_new,
};
use serde::{Deserialize, Serialize};
use std::io::Read;
use underskrift::CryptoSigner;
use zeroize::Zeroizing;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Panel {
    Identities,
    Appearance,
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
    #[serde(default)]
    appearance: Appearance,
}
/// The remembered look of a visible signature: library artwork and layout.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
struct Appearance {
    asset_id: Option<String>,
    layout: AppearanceLayout,
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
        format!("{} • {}", self.info.subject, self.source_label())
    }
    fn source_label(&self) -> &str {
        match &self.source {
            Source::File { alias, .. } => alias.as_str(),
            Source::Keychain { .. } => "macOS Keychain",
            Source::WindowsStore { .. } => "Windows Personal store",
            Source::Certificate => "public certificate",
        }
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
    layout: AppearanceLayout,
    /// Quick Sign is waiting for a box on the page.
    pub(super) quick: bool,
    /// Quick Sign needs the passphrase of a .p12/.pfx identity.
    quick_prompt: bool,
    /// The next asset added to the library becomes the signature artwork.
    pub(super) adopt_asset: bool,
    /// The appearance the current preview was rendered for.
    previewed: Option<(Option<String>, AppearanceLayout, bool)>,
    preview: Option<TextureHandle>,
    report: Option<(u64, u64, Vec<SignatureEvidence>, bool)>,
    trust_id: Option<String>,
    use_explicit_trust: bool,
    tx: mpsc::Sender<Event>,
    rx: mpsc::Receiver<Event>,
    /// Appearance previews render on their own channel so they never touch `busy`.
    appearance_tx: mpsc::Sender<Result<Vec<u8>, String>>,
    appearance_rx: mpsc::Receiver<Result<Vec<u8>, String>>,
    appearance_preview: Option<Result<TextureHandle, String>>,
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
            appearance,
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
        let (appearance_tx, appearance_rx) = mpsc::channel();
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
            asset_id: appearance.asset_id,
            layout: appearance.layout,
            quick: false,
            quick_prompt: false,
            adopt_asset: false,
            previewed: None,
            preview: None,
            report: None,
            trust_id: None,
            use_explicit_trust: false,
            tx,
            rx,
            appearance_tx,
            appearance_rx,
            appearance_preview: None,
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
            appearance: Appearance {
                asset_id: self.asset_id.clone(),
                layout: self.layout,
            },
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
    /// Remember the signature appearance for later sessions.
    fn save_appearance(&mut self) {
        if let Err(e) = self.save_catalogue() {
            self.error = Some(format!(
                "The signature appearance applies to this session, but could not be saved: {e:#}"
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
    /// Import an image or SVG and use it as the signature artwork.
    ImportArtwork,
    /// Draw or type a signature and use it as the signature artwork.
    DrawArtwork,
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
                    self.status = Some((
                        format!(
                            "Signed and saved as {} • your original is unchanged",
                            path.file_name().unwrap_or_default().to_string_lossy()
                        ),
                        false,
                    ));
                }
                Event::Failed(error) => {
                    if !self.digital.open {
                        self.status = Some((error.clone(), true));
                    }
                    self.digital.error = Some(error);
                }
            }
        }
        if self.digital.adopt_asset
            && let Some(id) = self.editor.last_added.take()
        {
            self.digital.adopt_asset = false;
            self.digital.asset_id = Some(id);
            self.digital.save_appearance();
            self.status = Some((
                "Signature image set for your digital signatures.".into(),
                false,
            ));
        }
        for result in self.digital.appearance_rx.try_iter().collect::<Vec<_>>() {
            self.digital.appearance_preview = Some(result.and_then(|png| {
                let image = mypdf_milestone0::assets::decode_png(&png)
                    .map_err(|e| format!("{e:#}"))?
                    .into_rgba8();
                Ok(ctx.load_texture(
                    "signature_appearance",
                    egui::ColorImage::from_rgba_unmultiplied(
                        [image.width() as usize, image.height() as usize],
                        &image,
                    ),
                    egui::TextureOptions::LINEAR,
                ))
            }));
        }
        if self.digital.open && self.digital.panel == Panel::Appearance {
            self.preview_appearance(ctx);
        }
        if self.editor.tool == editing::Tool::SignArea
            && !self.digital.quick
            && ctx.input(|i| i.key_pressed(egui::Key::Escape))
        {
            self.editor.select_tool(editing::Tool::Select);
            self.status = None;
        }
        if self.digital.quick {
            let cancel = ctx.input(|i| i.key_pressed(egui::Key::Escape));
            if cancel
                || self.editor.tool != editing::Tool::SignArea
                    && self.editor.pending_sign_rect.is_none()
            {
                self.digital.quick = false;
                if cancel {
                    self.editor.select_tool(editing::Tool::Select);
                }
            }
        }
        if let Some((tab, page, rect)) = self.editor.pending_sign_rect.take() {
            let rect = self.fit_signature_box(tab, page, rect);
            self.digital.document = Some(tab);
            self.digital.request.page = page;
            self.digital.request.rect = Some(rect);
            self.digital.visible = true;
            self.digital.preview = None;
            if std::mem::take(&mut self.digital.quick) {
                let file = self
                    .digital
                    .entries
                    .iter()
                    .find(|e| Some(&e.id) == self.digital.selected.as_ref())
                    .is_some_and(|e| matches!(e.source, Source::File { .. }));
                if file && self.digital.sign_password.is_empty() {
                    self.digital.quick_prompt = true;
                } else {
                    self.quick_sign_now(ctx);
                }
            } else {
                self.digital.open = true;
                self.digital.panel = Panel::Sign;
                self.status = None;
            }
        }
    }
    pub(super) fn digital_window(&mut self, ctx: &egui::Context) {
        self.quick_sign_prompt(ctx);
        if !self.digital.open {
            return;
        }
        let before = (
            self.digital.request.clone(),
            self.digital.asset_id.clone(),
            self.digital.visible,
        );
        let appearance = (self.digital.asset_id.clone(), self.digital.layout);
        let mut quick = false;
        let active = self.tabs.get(self.active);
        let mut open = true;
        let mut action = None;
        let assets: Vec<mypdf_milestone0::assets::Asset> = self
            .editor
            .library
            .as_ref()
            .map(|l| l.entries.clone())
            .unwrap_or_default();
        let default_entry = self
            .digital
            .entries
            .iter()
            .find(|e| e.eligible && self.digital.is_default(e))
            .map(Entry::label);
        egui::Window::new("Digital IDs and signatures")
            .open(&mut open)
            .collapsible(false)
            .vscroll(true)
            .max_height((ctx.content_rect().height() - 160.0).max(260.0))
            .default_width(610.0)
            .default_height(540.0)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 10.;
                const PANELS: [(Panel, &str); 4] = [
                    (Panel::Identities, "Digital IDs"),
                    (Panel::Appearance, "Appearance"),
                    (Panel::Sign, "Sign document"),
                    (Panel::Validate, "Validate"),
                ];
                let current = PANELS
                    .iter()
                    .position(|(p, _)| *p == self.digital.panel)
                    .unwrap_or(0);
                if let Some(i) = ui::segmented(ui, &PANELS.map(|(_, name)| name), current) {
                    self.digital.panel = PANELS[i].0;
                    self.digital.preview = None;
                }
                if self.digital.busy {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("Working… Input files are preserved.").color(theme::muted()));
                    });
                }
                for error in [&self.digital.error, &self.digital.catalogue_error]
                    .into_iter()
                    .flatten()
                {
                    ui::callout(ui, ui::Tone::Danger, error);
                }
                if !self.digital.message.is_empty() {
                    ui::callout(ui, ui::Tone::Success, &self.digital.message);
                }
                if self.digital.scanning {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new(format!("Loading {}…", store_label())).color(theme::muted()));
                    });
                }
                if let Some(error) = &self.digital.store_error {
                    ui::callout(ui, ui::Tone::Danger, error);
                }
                if self.digital.default_identity.is_some()
                    && !self
                        .digital
                        .entries
                        .iter()
                        .any(|e| e.eligible && self.digital.is_default(e))
                {
                    ui::callout(
                        ui,
                        ui::Tone::Warning,
                        "Your default certificate is unavailable. Refresh the store or choose another identity.",
                    );
                    if ui
                        .add(ActionButton::new(Icon::Close, "Clear default certificate"))
                        .clicked()
                    {
                        action = Some(Action::ClearDefault);
                    }
                }
                ui.add_enabled_ui(!self.digital.busy, |ui| match self.digital.panel {
                    Panel::Identities => {
                        ui.horizontal_wrapped(|ui| {
                            if ui
                                .add(ActionButton::new(Icon::Import, "Import .p12 / .pfx…"))
                                .clicked()
                            {
                                action = Some(Action::Import);
                            }
                            if ui
                                .add(ActionButton::new(Icon::Shield, "Create personal ID…"))
                                .clicked()
                            {
                                action = Some(Action::Create);
                            }
                            if ui
                                .add(ActionButton::new(Icon::Open, "Read .cer / .crt / .pem…"))
                                .clicked()
                            {
                                action = Some(Action::ReadCertificate);
                            }
                            #[cfg(any(target_os = "macos", target_os = "windows"))]
                            if ui
                                .add_enabled(
                                    !self.digital.scanning,
                                    ActionButton::new(Icon::Reload, "Refresh certificates"),
                                )
                                .clicked()
                            {
                                action = Some(Action::Keychain);
                            }
                        });
                        #[cfg(target_os = "macos")]
                        egui::CollapsingHeader::new("Other Keychain").show(ui, |ui| {
                            let r = ui::field(ui, "Keychain file", |ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.digital.keychain_path)
                                        .margin(ui::FIELD_MARGIN)
                                        .desired_width(f32::INFINITY)
                                        .hint_text("Empty uses the system search list"),
                                )
                            });
                            self.editor.text_editing |= r.has_focus();
                        });
                        ui.horizontal(|ui| {
                            ui::caption(ui, "Your identities");
                            if !self.digital.store_message.is_empty() {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            RichText::new(&self.digital.store_message)
                                                .size(11.5)
                                                .color(theme::muted()),
                                        );
                                    },
                                );
                            }
                        });
                        if self.digital.entries.is_empty() {
                            ui::callout(
                                ui,
                                ui::Tone::Info,
                                "Import a protected identity, create a personal ID, or select one from Keychain.",
                            );
                        } else if default_entry.is_none() {
                            ui::callout(
                                ui,
                                ui::Tone::Info,
                                "Select an identity and choose “Use as default certificate” to enable Quick Sign.",
                            );
                        }
                        ui.spacing_mut().item_spacing.y = 6.;
                        for entry in &self.digital.entries {
                            let selected = self.digital.selected.as_ref() == Some(&entry.id);
                            match identity_row(ui, entry, selected, self.digital.is_default(entry)) {
                                Some(RowClick::Select) => {
                                    self.digital.selected = Some(entry.id.clone())
                                }
                                Some(RowClick::Forget) => {
                                    action = Some(Action::Forget(entry.id.clone()))
                                }
                                None => {}
                            }
                        }
                        ui.spacing_mut().item_spacing.y = 10.;
                        if let Some(entry) = self
                            .digital
                            .entries
                            .iter()
                            .find(|e| Some(&e.id) == self.digital.selected.as_ref())
                        {
                            ui::section(ui, "Certificate", |ui| {
                                certificate_details(ui, entry);
                                if let Some(choice) = default_choice(ui, &self.digital, entry) {
                                    action = Some(choice);
                                }
                                ui.horizontal_wrapped(|ui| {
                                    if ui
                                        .add(ActionButton::new(
                                            Icon::Export,
                                            "Export public certificate…",
                                        ))
                                        .clicked()
                                    {
                                        action = Some(Action::ExportPublic);
                                    }
                                    if ui
                                        .add_enabled(
                                            matches!(entry.source, Source::File { .. }),
                                            ActionButton::new(Icon::Save, "Export encrypted .p12…"),
                                        )
                                        .clicked()
                                    {
                                        action = Some(Action::ExportP12);
                                    }
                                });
                            });
                        }
                        let mut cancel = false;
                        if let Some(import) = &mut self.digital.import {
                            ui::section(ui, "Unlock identity", |ui| {
                                let name = import
                                    .path
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .into_owned();
                                let r = ui::field(ui, &format!("Passphrase for {name}"), |ui| {
                                    ui.add(
                                        egui::TextEdit::singleline(&mut *import.password)
                                            .margin(ui::FIELD_MARGIN)
                                            .password(true)
                                            .desired_width(f32::INFINITY)
                                            .hint_text("Identity passphrase")
                                            .char_limit(1024),
                                    )
                                });
                                self.editor.text_editing |= r.has_focus();
                                ui.horizontal(|ui| {
                                    if ui
                                        .add(ActionButton::new(Icon::Check, "Unlock and import").primary())
                                        .clicked()
                                    {
                                        action = Some(Action::Unlock);
                                    }
                                    cancel = ui
                                        .add(ActionButton::new(Icon::Close, "Cancel import"))
                                        .clicked();
                                });
                            });
                        }
                        if cancel {
                            self.digital.import = None;
                        }
                        if self.digital.creating {
                            ui::section(ui, "Create personal digital ID", |ui| {
                                let mut focus = false;
                                egui::Grid::new("create_identity")
                                    .num_columns(2)
                                    .spacing(vec2(12., 8.))
                                    .show(ui, |ui| {
                                        for (label, value, hint, limit) in [
                                            ("Full name", &mut self.digital.create_name, "Jane Appleseed", 200),
                                            ("Organization", &mut self.digital.create_org, "Optional", 200),
                                            ("Email", &mut self.digital.create_email, "Optional", 254),
                                        ] {
                                            ui.label(RichText::new(label).color(theme::muted()));
                                            focus |= ui
                                                .add(
                                                    egui::TextEdit::singleline(value)
                                                        .margin(ui::FIELD_MARGIN)
                                                        .desired_width(f32::INFINITY)
                                                        .hint_text(hint)
                                                        .char_limit(limit),
                                                )
                                                .has_focus();
                                            ui.end_row();
                                        }
                                        ui.label(RichText::new("Valid for").color(theme::muted()));
                                        ui.horizontal(|ui| {
                                            ui.add(
                                                egui::DragValue::new(&mut self.digital.create_days)
                                                    .range(1..=3650)
                                                    .suffix(" days"),
                                            );
                                        });
                                        ui.end_row();
                                        ui.label(RichText::new("Key type").color(theme::muted()));
                                        ui.horizontal(|ui| {
                                            ui.selectable_value(&mut self.digital.create_ecdsa, false, "RSA 2048");
                                            ui.selectable_value(&mut self.digital.create_ecdsa, true, "P-256");
                                        });
                                        ui.end_row();
                                        for (label, value, hint) in [
                                            ("Passphrase", &mut self.digital.create_password, "At least 12 bytes"),
                                            ("Confirm", &mut self.digital.create_confirm, "Repeat the passphrase"),
                                        ] {
                                            ui.label(RichText::new(label).color(theme::muted()));
                                            focus |= ui
                                                .add(
                                                    egui::TextEdit::singleline(&mut **value)
                                                        .margin(ui::FIELD_MARGIN)
                                                        .password(true)
                                                        .desired_width(f32::INFINITY)
                                                        .hint_text(hint)
                                                        .char_limit(1024),
                                                )
                                                .has_focus();
                                            ui.end_row();
                                        }
                                    });
                                self.editor.text_editing |= focus;
                                ui.label(
                                    RichText::new("A self-signed ID signs cryptographically; it does not become trusted by Adobe or change trusted roots.")
                                        .small()
                                        .color(theme::muted()),
                                );
                                ui.horizontal(|ui| {
                                    if ui
                                        .add(ActionButton::new(Icon::Save, "Create and save .p12…").primary())
                                        .clicked()
                                    {
                                        action = Some(Action::CreateSave);
                                    }
                                    if ui.add(ActionButton::new(Icon::Close, "Cancel creation")).clicked() {
                                        self.digital.creating = false;
                                        self.digital.create_password = Zeroizing::new(String::new());
                                        self.digital.create_confirm = Zeroizing::new(String::new());
                                    }
                                });
                            });
                        }
                        let mut cancel = false;
                        if let Some(export) = &mut self.digital.export {
                            ui::section(ui, "Export encrypted identity backup", |ui| {
                                let mut focus = false;
                                for (label, value, hint) in [
                                    ("Current passphrase", &mut export.password, "Identity passphrase"),
                                    ("New backup passphrase", &mut export.new_password, "At least 12 bytes"),
                                    ("Confirm new passphrase", &mut export.confirm, "Repeat the new passphrase"),
                                ] {
                                    focus |= ui::field(ui, label, |ui| {
                                        ui.add(
                                            egui::TextEdit::singleline(&mut **value)
                                                .margin(ui::FIELD_MARGIN)
                                                .password(true)
                                                .desired_width(f32::INFINITY)
                                                .hint_text(hint)
                                                .char_limit(1024),
                                        )
                                    })
                                    .has_focus();
                                }
                                self.editor.text_editing |= focus;
                                ui.horizontal(|ui| {
                                    if ui
                                        .add(ActionButton::new(Icon::Save, "Export backup…").primary())
                                        .clicked()
                                    {
                                        action = Some(Action::ExportSave);
                                    }
                                    cancel = ui
                                        .add(ActionButton::new(Icon::Close, "Cancel export"))
                                        .clicked();
                                });
                            });
                        }
                        if cancel {
                            self.digital.export = None;
                        }
                    }
                    Panel::Appearance => {
                        ui.label(
                            RichText::new("Quick Sign draws this inside the box you place on the page. Your choice is remembered.")
                                .color(theme::muted()),
                        );
                        ui::section(ui, "Layout", |ui| {
                            layout_picker(ui, &mut self.digital.layout);
                        });
                        if self.digital.layout != AppearanceLayout::DetailsOnly {
                            ui::section(ui, "Signature image", |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    if ui.add(ActionButton::new(Icon::Pen, "Draw or type…")).clicked() {
                                        action = Some(Action::DrawArtwork);
                                    }
                                    if ui
                                        .add(ActionButton::new(Icon::Import, "Import image…"))
                                        .on_hover_text("PNG, JPEG or SVG")
                                        .clicked()
                                    {
                                        action = Some(Action::ImportArtwork);
                                    }
                                    if self.digital.asset_id.is_some()
                                        && self.digital.layout != AppearanceLayout::ImageOnly
                                        && ui.add(ActionButton::new(Icon::Close, "No image")).clicked()
                                    {
                                        self.digital.asset_id = None;
                                    }
                                });
                                if assets.is_empty() {
                                    ui::callout(
                                        ui,
                                        ui::Tone::Info,
                                        "Draw your signature or import an image to use it here.",
                                    );
                                }
                                ui.horizontal_wrapped(|ui| {
                                    ui.spacing_mut().item_spacing = vec2(8., 8.);
                                    // Newest first: a just-drawn signature is easy to find.
                                    for asset in assets.iter().rev().take(60) {
                                        let selected = self.digital.asset_id.as_ref() == Some(&asset.id);
                                        if artwork_card(ui, &mut self.editor, asset, selected) {
                                            self.digital.asset_id = Some(asset.id.clone());
                                        }
                                    }
                                });
                            });
                        }
                        ui::section(ui, "Preview", |ui| {
                            match &self.digital.appearance_preview {
                                _ if self.digital.layout == AppearanceLayout::ImageOnly
                                    && self.digital.asset_id.is_none() =>
                                {
                                    ui::callout(ui, ui::Tone::Info, "Choose a signature image to preview it.");
                                }
                                Some(Ok(image)) => {
                                    let size = image.size_vec2() / 2.;
                                    let size = size * (ui.available_width() / size.x).min(1.);
                                    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                                    if self.digital.layout == AppearanceLayout::ImageOnly {
                                        checkerboard(ui.painter(), rect);
                                    }
                                    ui.painter().image(
                                        image.id(),
                                        rect,
                                        Rect::from_min_max(pos2(0., 0.), pos2(1., 1.)),
                                        theme::surface(),
                                    );
                                }
                                Some(Err(e)) => ui::callout(ui, ui::Tone::Danger, e),
                                None => {
                                    ui.spinner();
                                }
                            }
                        });
                        match &default_entry {
                            Some(label) => ui::callout(
                                ui,
                                ui::Tone::Success,
                                &format!("Quick Sign signs with {label}."),
                            ),
                            None => ui::callout(
                                ui,
                                ui::Tone::Warning,
                                "Choose a default certificate under Digital IDs to use Quick Sign.",
                            ),
                        }
                        quick = ui
                            .add_enabled(
                                active.is_some_and(|t| t.bytes.is_some()),
                                ActionButton::new(Icon::Sign, "Quick Sign").primary(),
                            )
                            .on_hover_text("Drag a box on the page to sign")
                            .clicked();
                    }
                    Panel::Sign => {
                        ui::section(ui, "Document", |ui| {
                            if let Some(tab) = active {
                                ui.horizontal(|ui| {
                                    let (r, _) = ui.allocate_exact_size(vec2(18., 18.), egui::Sense::hover());
                                    ui::draw_icon(ui.painter(), r, Icon::Document, theme::danger());
                                    ui.label(RichText::new(tab.name()).family(ui::semibold()));
                                    if self.digital.document != Some(tab.id)
                                        && ui.add(ActionButton::new(Icon::Reload, "Use active document")).clicked()
                                    {
                                        self.digital.use_document(Some(tab));
                                    }
                                });
                            } else {
                                ui.label(RichText::new("Open a PDF to sign it.").color(theme::muted()));
                            }
                        });
                        ui::section(ui, "Signing identity", |ui| {
                            ui.horizontal(|ui| {
                                egui::ComboBox::from_id_salt("signing_id")
                                    .selected_text(
                                        self.digital
                                            .entries
                                            .iter()
                                            .find(|e| Some(&e.id) == self.digital.selected.as_ref())
                                            .map_or("Choose signing identity".into(), Entry::label),
                                    )
                                    .width((ui.available_width() - 44.).max(200.))
                                    .show_ui(ui, |ui| {
                                        for entry in self.digital.entries.iter().filter(|e| {
                                            e.eligible && !matches!(e.source, Source::Certificate)
                                        }) {
                                            if ui
                                                .selectable_value(
                                                    &mut self.digital.selected,
                                                    Some(entry.id.clone()),
                                                    entry.label(),
                                                )
                                                .clicked()
                                            {
                                                self.digital.request.name = entry.signer_name();
                                                self.digital.sign_password = Zeroizing::new(String::new());
                                                self.digital.preview = None;
                                                ui.close();
                                            }
                                        }
                                    });
                                #[cfg(any(target_os = "macos", target_os = "windows"))]
                                if ui
                                    .add_enabled(
                                        !self.digital.scanning,
                                        ActionButton::new(Icon::Reload, "Refresh certificates").compact(),
                                    )
                                    .clicked()
                                {
                                    action = Some(Action::Keychain);
                                }
                            });
                            if let Some(entry) = self
                                .digital
                                .entries
                                .iter()
                                .find(|e| Some(&e.id) == self.digital.selected.as_ref())
                            {
                                if let Some(choice) = default_choice(ui, &self.digital, entry) {
                                    action = Some(choice);
                                }
                                egui::CollapsingHeader::new("Certificate details")
                                    .show(ui, |ui| certificate_details(ui, entry));
                                if matches!(entry.source, Source::File { .. }) {
                                    let r = ui::field(ui, "Identity passphrase", |ui| {
                                        ui.add(
                                            egui::TextEdit::singleline(&mut *self.digital.sign_password)
                                                .margin(ui::FIELD_MARGIN)
                                                .password(true)
                                                .desired_width(f32::INFINITY)
                                                .char_limit(1024),
                                        )
                                    });
                                    self.editor.text_editing |= r.has_focus();
                                }
                            }
                        });
                        ui::section(ui, "Signature details", |ui| {
                            let mut focus = false;
                            egui::Grid::new("signature_details")
                                .num_columns(2)
                                .spacing(vec2(12., 8.))
                                .show(ui, |ui| {
                                    for (label, value, hint, limit) in [
                                        ("Field name", &mut self.digital.request.field, "New signature field", 100),
                                        ("Name", &mut self.digital.request.name, "Shown in the appearance", 200),
                                        ("Reason", &mut self.digital.request.reason, "Optional", 1000),
                                        ("Location", &mut self.digital.request.location, "Optional", 300),
                                    ] {
                                        ui.label(RichText::new(label).color(theme::muted()));
                                        focus |= ui
                                            .add(
                                                egui::TextEdit::singleline(value)
                                                    .margin(ui::FIELD_MARGIN)
                                                    .desired_width(f32::INFINITY)
                                                    .hint_text(hint)
                                                    .char_limit(limit),
                                            )
                                            .has_focus();
                                        ui.end_row();
                                    }
                                });
                            self.editor.text_editing |= focus;
                        });
                        ui::section(ui, "Appearance", |ui| {
                            ui.checkbox(&mut self.digital.visible, "Visible signature appearance");
                            if !self.digital.visible {
                                return;
                            }
                            layout_picker(ui, &mut self.digital.layout);
                            if let Some(tab) = active {
                                ui.horizontal(|ui| {
                                    let mut page = self.digital.request.page + 1;
                                    ui.label(RichText::new("Page").color(theme::muted()));
                                    if ui
                                        .add(egui::DragValue::new(&mut page).range(1..=tab.pages.len().max(1)))
                                        .changed()
                                    {
                                        self.digital.request.page = page - 1;
                                        self.digital.preview = None;
                                    }
                                    if ui
                                        .add(ActionButton::new(Icon::Rectangle, "Draw signature rectangle on page"))
                                        .clicked()
                                    {
                                        action = Some(Action::Place);
                                    }
                                });
                            }
                            if let Some(r) = &mut self.digital.request.rect {
                                egui::Grid::new("signature_rect").num_columns(4).show(ui, |ui| {
                                    let mut x = r[0];
                                    let mut y = r[1];
                                    let mut w = r[2] - r[0];
                                    let mut h = r[3] - r[1];
                                    for (i, (name, value)) in [("Left", &mut x), ("Top", &mut y), ("Width", &mut w), ("Height", &mut h)]
                                        .into_iter()
                                        .enumerate()
                                    {
                                        ui.label(RichText::new(name).color(theme::muted()));
                                        ui.add(egui::DragValue::new(value).range(0.0..=1500.0).speed(1.0).suffix(" pt"));
                                        if i % 2 == 1 {
                                            ui.end_row();
                                        }
                                    }
                                    *r = [x, y, x + w, y + h];
                                });
                            }
                            ui::field(ui, "Artwork", |ui| {
                                egui::ComboBox::from_id_salt("signature_artwork")
                                    .selected_text(
                                        self.digital
                                            .asset_id
                                            .as_ref()
                                            .and_then(|id| assets.iter().find(|a| &a.id == id))
                                            .map_or("No artwork", |a| a.name.as_str()),
                                    )
                                    .width(ui.available_width())
                                    .show_ui(ui, |ui| {
                                        if ui.selectable_value(&mut self.digital.asset_id, None, "No artwork").clicked() {
                                            ui.close();
                                        }
                                        for asset in assets {
                                            if ui
                                                .selectable_value(&mut self.digital.asset_id, Some(asset.id.clone()), &asset.name)
                                                .clicked()
                                            {
                                                ui.close();
                                            }
                                        }
                                    });
                            });
                            if ui.add(ActionButton::new(Icon::Read, "Preview appearance")).clicked() {
                                action = Some(Action::Preview);
                            }
                            if let Some(image) = &self.digital.preview {
                                let size = image.size_vec2();
                                egui::Frame::new()
                                    .fill(Color32::WHITE)
                                    .stroke(Stroke::new(1., theme::line()))
                                    .corner_radius(8)
                                    .inner_margin(8)
                                    .show(ui, |ui| {
                                        ui.add(egui::Image::new(image).fit_to_exact_size(vec2(
                                            480.0_f32.min(size.x / 2.0),
                                            (size.y / size.x * 480.0).min(220.0),
                                        )));
                                    });
                            }
                        });
                        if let Some(tab) = active {
                            if let Some(caps) = &tab.capabilities {
                                if caps.certified {
                                    ui::callout(ui, ui::Tone::Danger, "Certification restrictions are unsupported; this PDF cannot be extended here.");
                                }
                                if caps.needs_signing_preparation {
                                    if caps.signed {
                                        ui::callout(ui, ui::Tone::Danger, "This signed PDF uses xref streams. It cannot be rewritten for this writer.");
                                    } else {
                                        ui.checkbox(&mut self.digital.prepare, "Prepare an unsigned signing copy (required for this PDF)");
                                    }
                                }
                            }
                            if tab.session.as_ref().is_some_and(|s| s.revision() != 0) {
                                ui::callout(ui, ui::Tone::Info, "Current annotations will be included in the signed copy.");
                            }
                        }
                        ui.label(
                            RichText::new("New approval signature • offline verification • local signing time only. Save to a new filename.")
                                .small()
                                .color(theme::muted()),
                        );
                        let usable = active.is_some_and(|t| {
                            t.bytes.is_some()
                                && self.digital.document == Some(t.id)
                                && t.capabilities.as_ref().is_some_and(|c| {
                                    !(c.certified || c.encrypted || c.signed && c.needs_signing_preparation)
                                })
                        }) && self.digital.entries.iter().any(|e| {
                            Some(&e.id) == self.digital.selected.as_ref()
                                && e.eligible
                                && !matches!(e.source, Source::Certificate)
                        });
                        if ui
                            .add_enabled(usable, ActionButton::new(Icon::Sign, "Sign and save a copy…").primary())
                            .clicked()
                        {
                            action = Some(Action::Sign);
                        }
                    }
                    Panel::Validate => {
                        ui::section(ui, "Document", |ui| {
                            if let Some(tab) = active {
                                ui.horizontal(|ui| {
                                    let (r, _) = ui.allocate_exact_size(vec2(18., 18.), egui::Sense::hover());
                                    ui::draw_icon(ui.painter(), r, Icon::Document, theme::danger());
                                    ui.label(RichText::new(tab.name()).family(ui::semibold()));
                                });
                            } else {
                                ui.label(RichText::new("Open a PDF to validate its signatures.").color(theme::muted()));
                            }
                            ui.checkbox(
                                &mut self.digital.use_explicit_trust,
                                "Verify with an explicitly selected certificate for this session",
                            );
                            if self.digital.use_explicit_trust {
                                egui::ComboBox::from_id_salt("verify_cert")
                                    .selected_text(
                                        self.digital
                                            .entries
                                            .iter()
                                            .find(|e| Some(&e.id) == self.digital.trust_id.as_ref())
                                            .map_or("Choose public certificate".into(), Entry::label),
                                    )
                                    .width(ui.available_width())
                                    .show_ui(ui, |ui| {
                                        for entry in &self.digital.entries {
                                            if ui
                                                .selectable_value(&mut self.digital.trust_id, Some(entry.id.clone()), entry.label())
                                                .clicked()
                                            {
                                                ui.close();
                                            }
                                        }
                                    });
                            }
                        });
                        if ui
                            .add_enabled(
                                active.is_some_and(|t| t.bytes.is_some()),
                                ActionButton::new(Icon::Shield, "Validate signatures").primary(),
                            )
                            .clicked()
                        {
                            action = Some(Action::Verify);
                        }
                        ui.label(
                            RichText::new("Integrity, certificate trust and later revisions are separate checks. Revocation and timestamp trust are not checked online. Trusted roots are never modified.")
                                .small()
                                .color(theme::muted()),
                        );
                        if let Some((id, revision, report, trust)) = &self.digital.report {
                            if active.is_none_or(|t| {
                                t.id != *id || t.session.as_ref().map_or(0, |s| s.revision()) != *revision
                            }) {
                                ui::callout(ui, ui::Tone::Warning, "This report belongs to another document/revision. Validate the active document again.");
                            } else {
                                if report.is_empty() {
                                    ui::callout(ui, ui::Tone::Info, "No applied digital signatures were found.");
                                }
                                for sig in report {
                                    ui::section(ui, &sig.field, |ui| {
                                        if let Some(info) = &sig.signer {
                                            ui.label(RichText::new(&info.subject).family(ui::semibold()));
                                        }
                                        ui.spacing_mut().item_spacing.y = 6.;
                                        ui::check_row(ui, "Signature mathematics", sig.crypto_valid);
                                        ui::check_row(ui, "Signed content digest", sig.digest_matches);
                                        ui::check_row(ui, "Signature byte range", sig.byte_range_valid);
                                        ui::check_row(ui, "Certificate binding", sig.certificate_binding_valid);
                                        ui.add_space(4.);
                                        egui::Grid::new((&sig.field, "signature_facts"))
                                            .num_columns(2)
                                            .spacing(vec2(12., 6.))
                                            .show(ui, |ui| {
                                                let timestamp = sig.timestamp.as_ref().map(|t| {
                                                    format!("{t} (trust not independently evaluated here)")
                                                });
                                                for (label, value) in [
                                                    (
                                                        "Identity trust",
                                                        if sig.trusted && *trust {
                                                            "Trusted against the selected certificate for this session"
                                                        } else {
                                                            "Unknown / untrusted"
                                                        }
                                                        .to_owned(),
                                                    ),
                                                    (
                                                        "Coverage",
                                                        if sig.covers_current_file {
                                                            "Current file"
                                                        } else {
                                                            "Signed revision; later bytes exist"
                                                        }
                                                        .to_owned(),
                                                    ),
                                                    (
                                                        "Claimed signing time",
                                                        sig.claimed_signing_time
                                                            .clone()
                                                            .unwrap_or_else(|| "Not provided".into()),
                                                    ),
                                                    ("Certificate validity", sig.certificate_validity.clone()),
                                                    ("Revocation", "Not checked (offline)".into()),
                                                ]
                                                .into_iter()
                                                .chain(timestamp.map(|t| ("Reported timestamp", t)))
                                                {
                                                    ui.label(RichText::new(label).color(theme::muted()));
                                                    ui.add(egui::Label::new(value).wrap());
                                                    ui.end_row();
                                                }
                                            });
                                        if let Some(info) = &sig.signer {
                                            ui.collapsing("Signing certificate", |ui| certificate_info(ui, info));
                                        }
                                    });
                                }
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
        if appearance != (self.digital.asset_id.clone(), self.digital.layout) {
            self.digital.preview = None;
            self.digital.save_appearance();
        }
        if quick {
            self.quick_sign();
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
    /// Size a placed box for the current layout. A click becomes a default-size box
    /// centered on the click; a small drag grows to the layout's minimum.
    fn fit_signature_box(&self, tab: u64, page: usize, rect: [f32; 4]) -> [f32; 4] {
        let (min_w, min_h) = self.digital.layout.minimum_size();
        let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
        let click = w < 8. && h < 8.;
        let (mut w, mut h) = if click {
            match self.digital.layout {
                AppearanceLayout::ImageOnly => {
                    let aspect = self.signature_artwork().map_or(3., |a| a.aspect);
                    (160., (160. / aspect).clamp(min_h, 120.))
                }
                _ => (240., 72.),
            }
        } else {
            (w.max(min_w), h.max(min_h))
        };
        let Some(size) = self
            .tabs
            .iter()
            .find(|t| t.id == tab)
            .and_then(|t| t.pages.get(page).copied())
        else {
            return rect;
        };
        w = w.min(size.width);
        h = h.min(size.height);
        let (x, y) = if click {
            (rect[0] - w / 2., rect[1] - h / 2.)
        } else {
            (rect[0], rect[1])
        };
        let x = x.clamp(0., size.width - w);
        let y = y.clamp(0., size.height - h);
        [x, y, x + w, y + h]
    }
    /// Render the remembered appearance with sample details when it changes.
    fn preview_appearance(&mut self, ctx: &egui::Context) {
        let key = (self.digital.asset_id.clone(), self.digital.layout, true);
        if self.digital.previewed.as_ref() == Some(&key) {
            return;
        }
        self.digital.previewed = Some(key);
        let artwork = self.signature_artwork();
        let layout = self.digital.layout;
        if layout == AppearanceLayout::ImageOnly && artwork.is_none() {
            self.digital.appearance_preview = None;
            return;
        }
        let name = self
            .digital
            .entries
            .iter()
            .find(|e| e.eligible && self.digital.is_default(e))
            .map_or_else(|| "Your Name".to_owned(), Entry::signer_name);
        let (w, h) = match layout {
            AppearanceLayout::ImageOnly => {
                let aspect = artwork.as_ref().map_or(3., |a| a.aspect);
                (220., (220. / aspect).clamp(24., 110.))
            }
            _ => (300., 90.),
        };
        let request = DesktopSigningRequest {
            rect: Some([0., 0., w, h]),
            name,
            artwork,
            layout,
            ..Default::default()
        };
        let tx = self.digital.appearance_tx.clone();
        let wake = ctx.clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(|| signing::signature_artwork(&request))
                .map_err(|_| "The preview could not be drawn.".to_string())
                .and_then(|r| r.map_err(|e| format!("{e:#}")));
            let _ = tx.send(result);
            wake.request_repaint();
        });
    }
    /// The library artwork chosen for the signature appearance.
    fn signature_artwork(&self) -> Option<mypdf_milestone0::assets::Asset> {
        let id = self.digital.asset_id.as_ref()?;
        self.editor
            .library
            .as_ref()?
            .entries
            .iter()
            .find(|a| &a.id == id)
            .cloned()
    }
    /// Sign the active document with the selected identity and write `path`,
    /// which must not exist yet.
    fn sign_to(&mut self, path: PathBuf, ctx: &egui::Context) {
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
            self.digital.error = Some("Choose the active document again before signing.".into());
            return;
        }
        if self.digital.busy {
            self.status = Some(("Wait for the current signing task to finish.".into(), false));
            return;
        }
        let tab_id = tab.id;
        let revision = tab.session.as_ref().map_or(0, |s| s.revision());
        let annotations = tab
            .session
            .as_ref()
            .map(|s| s.annotations.clone())
            .unwrap_or_default();
        let signed_original = tab.signed_original.clone();
        // Preparing rewrites the file, which an existing signature cannot survive.
        let prepare = self.digital.prepare && signed_original.is_none();
        let mut request = self.digital.request.clone();
        if !self.digital.visible {
            request.rect = None;
        }
        request.artwork = self.signature_artwork();
        request.layout = self.digital.layout;
        let password = std::mem::take(&mut self.digital.sign_password);
        self.status = Some(("Signing…".into(), false));
        self.digital.spawn(ctx, move || {
            let mut input = if revision == 0 {
                bytes.to_vec()
            } else {
                mypdf_milestone0::annotations::export(&bytes, &annotations)?
            };
            if let Some(original) = signed_original {
                input = mypdf_milestone0::signature_edit::incremental_update(&original, &input)?;
            }
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
    /// Quick Sign: check the default ID and document, then wait for a box on the page.
    /// Why the active document cannot be signed here, if it cannot.
    fn signing_problem(&mut self) -> bool {
        let Some(tab) = self.tabs.get(self.active).filter(|t| t.bytes.is_some()) else {
            self.status = Some(("Open a PDF to sign it.".into(), false));
            return true;
        };
        let problem = match &tab.capabilities {
            Some(c) if c.certified => Some(
                "Certification restrictions are unsupported; this PDF cannot be extended here.",
            ),
            Some(c) if c.encrypted => Some("Encrypted PDFs cannot be signed here."),
            Some(c) if c.signed && c.needs_signing_preparation => {
                Some("This signed PDF uses xref streams. It cannot be rewritten for this writer.")
            }
            None => Some("The document is still loading."),
            _ => None,
        };
        if let Some(problem) = problem {
            self.status = Some((problem.into(), true));
        }
        problem.is_some()
    }
    /// Digital Signature: first pick the area on the page, then choose the ID and details.
    pub(super) fn sign_in_area(&mut self) {
        if self.signing_problem() || !self.apply_pending_annotation() {
            return;
        }
        let tab = &self.tabs[self.active];
        self.digital.use_document(Some(tab));
        self.digital.document = Some(tab.id);
        self.digital.prepare = tab
            .capabilities
            .as_ref()
            .is_some_and(|c| c.needs_signing_preparation);
        self.digital.open = false;
        self.digital.quick = false;
        self.editor.mode = editing::Mode::Sign;
        self.editor.start_signature_rectangle();
        self.status = Some((
            "Drag a box where the signature goes, or click to place it · Esc cancels".into(),
            false,
        ));
    }
    pub(super) fn quick_sign(&mut self) {
        if self.signing_problem() {
            return;
        }
        let tab = &self.tabs[self.active];
        let prepare = tab
            .capabilities
            .as_ref()
            .is_some_and(|c| c.needs_signing_preparation);
        let Some(entry) = self
            .digital
            .entries
            .iter()
            .find(|e| e.eligible && self.digital.is_default(e))
            .cloned()
        else {
            self.digital.open = true;
            self.digital.panel = Panel::Identities;
            self.digital.error = Some(if self.digital.entries.iter().any(|e| e.eligible) {
                "Quick Sign needs a default certificate: select an identity, then choose “Use as default certificate”.".into()
            } else {
                "Quick Sign needs a digital ID: import a .p12/.pfx, create a personal ID, or refresh certificates.".into()
            });
            return;
        };
        if self.digital.layout == AppearanceLayout::ImageOnly && self.signature_artwork().is_none()
        {
            self.digital.open = true;
            self.digital.panel = Panel::Appearance;
            self.digital.error = Some(
                "Choose a signature image, or switch to a layout with signing details.".into(),
            );
            return;
        }
        if !self.apply_pending_annotation() {
            return;
        }
        let tab = &self.tabs[self.active];
        self.digital.use_document(Some(tab));
        self.digital.document = Some(tab.id);
        self.digital.prepare = prepare;
        self.digital.visible = true;
        self.digital.selected = Some(entry.id.clone());
        self.digital.request.name = entry.signer_name();
        self.digital.open = false;
        self.digital.quick = true;
        self.editor.mode = editing::Mode::Sign;
        self.editor.start_signature_rectangle();
        self.status = None;
    }
    /// Sign the placed box at once, next to the original as “Name-signed.pdf”.
    fn quick_sign_now(&mut self, ctx: &egui::Context) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let directory = tab
            .path
            .parent()
            .filter(|d| d.is_absolute() && d.is_dir())
            .map(std::path::Path::to_path_buf);
        let stem = tab
            .path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        // An unsaved document has no folder yet: ask where to save it.
        let Some(directory) = directory else {
            self.digital_action(Action::Sign, ctx);
            return;
        };
        let path = (1..1000)
            .map(|n| {
                directory.join(if n == 1 {
                    format!("{stem}-signed.pdf")
                } else {
                    format!("{stem}-signed {n}.pdf")
                })
            })
            .find(|p| !p.exists());
        match path {
            Some(path) => self.sign_to(path, ctx),
            None => self.digital_action(Action::Sign, ctx),
        }
    }
    /// Passphrase prompt for Quick Sign with a .p12/.pfx identity.
    fn quick_sign_prompt(&mut self, ctx: &egui::Context) {
        if !self.digital.quick_prompt {
            return;
        }
        let name = self
            .digital
            .entries
            .iter()
            .find(|e| Some(&e.id) == self.digital.selected.as_ref())
            .map(|e| e.source_label().to_owned())
            .unwrap_or_default();
        let mut decision = None;
        let modal = egui::Modal::new(egui::Id::new("quick_sign_passphrase"))
            .backdrop_color(Color32::from_black_alpha(64))
            .frame(ui::modal_frame())
            .show(ctx, |ui| {
                ui.set_width(300.);
                ui.vertical_centered(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(54., 54.), egui::Sense::hover());
                    ui::badge(
                        ui.painter(),
                        r,
                        Icon::Shield,
                        ui::Fill::Brand,
                        Color32::WHITE,
                    );
                    ui.add_space(12.);
                    ui::title(ui, "Sign with your digital ID", 16.);
                    ui.label(
                        RichText::new(format!("Enter the passphrase for {name}."))
                            .size(13.)
                            .color(theme::muted()),
                    );
                });
                ui.add_space(12.);
                let r = ui.add(
                    egui::TextEdit::singleline(&mut *self.digital.sign_password)
                        .margin(ui::FIELD_MARGIN)
                        .password(true)
                        .desired_width(f32::INFINITY)
                        .hint_text("Passphrase")
                        .char_limit(1024),
                );
                r.request_focus();
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    decision = Some(true);
                }
                ui.add_space(14.);
                let half = (ui.available_width() - 10.) / 2.;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.;
                    ui.allocate_ui(vec2(half, 32.), |ui| {
                        if ui::dialog_button(ui, "Cancel", ui::DialogTone::Secondary).clicked() {
                            decision = Some(false);
                        }
                    });
                    ui.allocate_ui(vec2(half, 32.), |ui| {
                        if ui::dialog_button(ui, "Sign", ui::DialogTone::Primary).clicked() {
                            decision = Some(true);
                        }
                    });
                });
            });
        if modal.should_close() {
            decision = decision.or(Some(false));
        }
        match decision {
            Some(true) => {
                self.digital.quick_prompt = false;
                self.quick_sign_now(ctx);
            }
            Some(false) => {
                self.digital.quick_prompt = false;
                self.digital.sign_password = Zeroizing::new(String::new());
            }
            None => {}
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
                request.layout = self.digital.layout;
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
            Action::ImportArtwork => {
                self.editor.last_added = None;
                self.digital.adopt_asset = true;
                self.import_library_asset(ctx, None, false);
            }
            Action::DrawArtwork => {
                self.editor.last_added = None;
                self.digital.adopt_asset = true;
                self.editor.signature_open = true;
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
                let Some(tab) = self.tabs.get(self.active) else {
                    return;
                };
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
                    self.sign_to(path, ctx);
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

#[cfg(test)]
impl DigitalState {
    /// Visual QA: sample Keychain, file and certificate identities.
    pub(super) fn preview_identities(&mut self, panel: Panel) {
        let info = |subject: &str, until: &str| IdentityInfo {
            subject: subject.into(),
            issuer: subject.into(),
            not_before: "2026-01-04 09:12:00 UTC".into(),
            not_after: until.into(),
            sha256: "4F:2A:91:0C:7B:E3:55:18:9D:A0:6C:21:F8:3E:B7:42".into(),
            algorithm: "ECDSA P-256 with SHA-256".into(),
            self_issued: true,
        };
        let entries = [
            (
                Source::Keychain {
                    path: None,
                    fingerprint: "a1".into(),
                },
                info("CN=Jane Appleseed, O=Example Studio", "2028-01-04"),
                true,
            ),
            (
                Source::File {
                    path: "/tmp/jane.p12".into(),
                    alias: "jane.p12".into(),
                },
                info("CN=Jane Appleseed (backup)", "2027-06-30"),
                true,
            ),
            (
                Source::Certificate,
                info("CN=Partner Signing CA", "2026-03-01"),
                false,
            ),
        ];
        self.entries = entries
            .into_iter()
            .enumerate()
            .map(|(i, (source, info, eligible))| Entry {
                id: format!("{i:032x}"),
                source,
                info,
                der: vec![],
                eligible,
                explanation: (!eligible)
                    .then(|| "Public certificates can verify signatures but cannot sign.".into()),
            })
            .collect();
        self.default_identity = Some(self.entries[0].reference());
        self.selected = Some(self.entries[0].id.clone());
        self.message = "Default signing certificate saved.".into();
        self.panel = panel;
        self.open = true;
    }
    /// Visual QA: Quick Sign asking for a .p12 passphrase.
    pub(super) fn preview_quick_prompt(&mut self) {
        self.preview_identities(Panel::Sign);
        self.selected = Some(self.entries[1].id.clone());
        self.open = false;
        self.quick_prompt = true;
    }
    /// Visual QA: use an artwork with a layout, and report when its preview is drawn.
    pub(super) fn preview_appearance(&mut self, asset: String, layout: AppearanceLayout) {
        self.asset_id = Some(asset);
        self.layout = layout;
    }
    pub(super) fn appearance_ready(&self) -> bool {
        self.appearance_preview.is_some()
    }
}

/// The three appearance layouts as a segmented control, with a one-line description.
fn layout_picker(ui: &mut egui::Ui, layout: &mut AppearanceLayout) {
    const LAYOUTS: [(AppearanceLayout, &str, &str); 3] = [
        (
            AppearanceLayout::ImageAndDetails,
            "Image + details",
            "Your image beside “Digitally signed by”, your name and the date.",
        ),
        (
            AppearanceLayout::ImageOnly,
            "Image only",
            "Just your image, such as a handwritten signature. The page shows through around it.",
        ),
        (
            AppearanceLayout::DetailsOnly,
            "Details only",
            "“Digitally signed by”, your name and the date in a bordered box.",
        ),
    ];
    let current = LAYOUTS.iter().position(|l| l.0 == *layout).unwrap_or(0);
    if let Some(i) = ui::segmented(ui, &LAYOUTS.map(|l| l.1), current) {
        *layout = LAYOUTS[i].0;
    }
    let description = LAYOUTS.iter().find(|l| l.0 == *layout).map_or("", |l| l.2);
    ui.label(RichText::new(description).size(12.).color(theme::muted()));
}
/// A selectable library artwork tile. Returns true when clicked.
fn artwork_card(
    ui: &mut egui::Ui,
    editor: &mut editing::EditorState,
    asset: &mypdf_milestone0::assets::Asset,
    selected: bool,
) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(112., 78.), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Button,
            true,
            selected,
            asset.name.as_str(),
        )
    });
    let hovered = response.hovered();
    ui.painter().rect(
        rect,
        10,
        if selected {
            theme::accent_soft()
        } else {
            theme::surface()
        },
        Stroke::new(
            if selected { 2. } else { 1. },
            if selected {
                theme::accent()
            } else if hovered {
                theme::accent_line()
            } else {
                theme::line()
            },
        ),
        egui::StrokeKind::Inside,
    );
    let art = Rect::from_min_max(rect.min + vec2(10., 8.), rect.max - vec2(10., 24.));
    // Artwork sits on white paper in both themes.
    ui.painter()
        .rect_filled(art.expand2(vec2(4., 2.)), 5, Color32::WHITE);
    editing::paint_asset(editor, ui.ctx(), ui.painter(), art, asset);
    let mut job = egui::text::LayoutJob::single_section(
        asset.name.clone(),
        egui::TextFormat::simple(FontId::proportional(11.), theme::muted()),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - 14.);
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(
        pos2(rect.center().x - galley.size().x / 2., rect.bottom() - 18.),
        galley,
        theme::muted(),
    );
    response
        .on_hover_text(&asset.name)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}
/// The usual transparency backdrop.
fn checkerboard(painter: &egui::Painter, rect: Rect) {
    painter.rect_filled(rect, 4, Color32::WHITE);
    let cell = 8.;
    let painter = painter.with_clip_rect(rect);
    for row in 0..(rect.height() / cell).ceil() as usize {
        for column in 0..(rect.width() / cell).ceil() as usize {
            if (row + column) % 2 == 1 {
                painter.rect_filled(
                    Rect::from_min_size(
                        rect.min + vec2(column as f32 * cell, row as f32 * cell),
                        vec2(cell, cell),
                    ),
                    0,
                    Color32::from_gray(238),
                );
            }
        }
    }
}

enum RowClick {
    Select,
    Forget,
}
/// An identity card: shield tile, subject, source and default badge. Stored file
/// and certificate references can be forgotten from the row.
fn identity_row(
    ui: &mut egui::Ui,
    entry: &Entry,
    selected: bool,
    default: bool,
) -> Option<RowClick> {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), 52.), egui::Sense::click());
    let label = entry.label();
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, label.as_str())
    });
    let forgettable = !matches!(
        entry.source,
        Source::Keychain { .. } | Source::WindowsStore { .. }
    );
    let forget_rect =
        Rect::from_center_size(pos2(rect.right() - 24., rect.center().y), vec2(32., 32.));
    let painter = ui.painter();
    let hovered = response.hovered();
    painter.rect(
        rect,
        10,
        if selected {
            theme::accent_soft()
        } else if hovered {
            theme::paper()
        } else {
            theme::surface()
        },
        Stroke::new(
            1.,
            if selected {
                theme::accent_line()
            } else {
                theme::line()
            },
        ),
        egui::StrokeKind::Inside,
    );
    let tile = Rect::from_center_size(pos2(rect.left() + 28., rect.center().y), vec2(32., 32.));
    ui::badge(
        painter,
        tile,
        if matches!(entry.source, Source::Certificate) {
            Icon::Document
        } else {
            Icon::Shield
        },
        if entry.eligible {
            ui::Fill::Brand
        } else {
            ui::Fill::Sheen
        },
        if entry.eligible {
            Color32::WHITE
        } else {
            Color32::from_gray(190)
        },
    );
    let text_left = rect.left() + 54.;
    let right = if forgettable {
        forget_rect.left() - 8.
    } else {
        rect.right() - 12.
    };
    let badge_width = if default { 64. } else { 0. };
    let elide = |text: &str, font: FontId, color: Color32, width: f32| {
        let mut job = egui::text::LayoutJob::single_section(
            text.to_owned(),
            egui::TextFormat::simple(font, color),
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(40.));
        ui.painter().layout_job(job)
    };
    let title = elide(
        &entry.info.subject,
        ui::strong(ui, 13.5),
        if entry.eligible {
            theme::ink()
        } else {
            theme::muted()
        },
        right - text_left - badge_width,
    );
    let detail = if entry.eligible {
        format!(
            "{} · valid until {}",
            entry.source_label(),
            entry.info.not_after
        )
    } else {
        format!("{} · unavailable", entry.source_label())
    };
    let subtitle = elide(
        &detail,
        FontId::proportional(11.5),
        theme::muted(),
        right - text_left,
    );
    let title_width = title.size().x;
    painter.galley(pos2(text_left, rect.center().y - 17.), title, theme::ink());
    painter.galley(
        pos2(text_left, rect.center().y + 2.),
        subtitle,
        theme::muted(),
    );
    if default {
        let pill = Rect::from_min_size(
            pos2(text_left + title_width + 8., rect.center().y - 16.),
            vec2(56., 17.),
        );
        painter.rect_filled(pill, 8., theme::success().gamma_multiply(0.14));
        painter.text(
            pill.center(),
            egui::Align2::CENTER_CENTER,
            "Default",
            ui::strong(ui, 10.5),
            theme::success(),
        );
    }
    if response.has_focus() {
        painter.rect_stroke(
            rect,
            10,
            Stroke::new(2.5, theme::accent()),
            egui::StrokeKind::Inside,
        );
    }
    if forgettable
        && ui
            .put(
                forget_rect,
                ActionButton::new(Icon::Trash, "Forget reference").compact(),
            )
            .clicked()
    {
        return Some(RowClick::Forget);
    }
    response
        .on_hover_text(label)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
        .then_some(RowClick::Select)
}

fn default_choice(ui: &mut egui::Ui, state: &DigitalState, entry: &Entry) -> Option<Action> {
    if state.is_default(entry) {
        let mut clear = false;
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(16., 16.), egui::Sense::hover());
            ui::draw_icon(ui.painter(), r, Icon::Check, theme::success());
            ui.label(RichText::new("Default signing certificate").color(theme::success()));
            clear = ui.link("Clear default").clicked();
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
        ui.label(RichText::new(explanation).small().color(theme::muted()));
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
                .color(theme::muted()),
        );
    }
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
    fn digital_signature_asks_for_the_area_before_the_signing_window() {
        use eframe::App;
        let temp = tempfile::tempdir().unwrap();
        let pdf = temp.path().join("Area.pdf");
        let font = mypdf_milestone0::document::default_font().unwrap();
        std::fs::write(&pdf, mypdf_milestone0::document::fixture(&font).unwrap()).unwrap();
        let ctx = egui::Context::default();
        let mut app = Viewer::new(
            &eframe::CreationContext::_new_kittest(ctx.clone()),
            vec![pdf],
        );
        app.digital = DigitalState::at(temp.path().join("profile"));
        let frame = |app: &mut Viewer, events: Vec<egui::Event>| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1100., 760.))),
                    events,
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            );
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while app.tabs[0].capabilities.is_none() {
            assert!(std::time::Instant::now() < deadline);
            frame(&mut app, vec![]);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let escape = egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        app.sign_in_area();
        assert!(app.editor.tool == editing::Tool::SignArea && !app.digital.open);
        frame(&mut app, vec![escape]);
        assert!(app.editor.tool != editing::Tool::SignArea, "Esc cancels");

        app.sign_in_area();
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        let page = ctx
            .read_response(egui::Id::new((app.tabs[0].id, 0_usize, "page_actions")))
            .unwrap()
            .rect;
        let at = page.min + vec2(300., 300.) * app.tabs[0].scale;
        for pressed in [true, false] {
            frame(
                &mut app,
                vec![
                    egui::Event::PointerMoved(at),
                    egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert!(app.digital.open && app.digital.panel == Panel::Sign);
        assert!(!app.digital.quick && app.digital.visible);
        let r = app.digital.request.rect.expect("the area is filled in");
        assert!(((r[0] + r[2]) / 2. - 300.).abs() < 0.5 && ((r[1] + r[3]) / 2. - 300.).abs() < 0.5);
    }
    #[test]
    fn quick_sign_places_a_box_and_saves_a_verified_copy_beside_the_original() {
        use eframe::App;
        let temp = tempfile::tempdir().unwrap();
        let passphrase = "synthetic-quick-passphrase";
        let created = identity::create("Quick Signer", passphrase, true).unwrap();
        let file = temp.path().join("quick.p12");
        std::fs::write(&file, &created.pkcs12).unwrap();
        let imported = identity::inspect_pkcs12(&created.pkcs12, passphrase)
            .unwrap()
            .remove(0);
        let entry = Entry::file(file, imported);
        let pdf = temp.path().join("Contract.pdf");
        let font = mypdf_milestone0::document::default_font().unwrap();
        std::fs::write(&pdf, mypdf_milestone0::document::fixture(&font).unwrap()).unwrap();
        // An earlier signed copy is never replaced; the next free name is used.
        std::fs::write(temp.path().join("Contract-signed.pdf"), b"keep").unwrap();

        let ctx = egui::Context::default();
        let mut app = Viewer::new(
            &eframe::CreationContext::_new_kittest(ctx.clone()),
            vec![pdf],
        );
        app.digital = DigitalState::at(temp.path().join("profile"));
        let frame = |app: &mut Viewer, events: Vec<egui::Event>| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1100., 760.))),
                    events,
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            );
        };
        let until = |app: &mut Viewer, ready: &dyn Fn(&Viewer) -> bool| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !ready(app) {
                assert!(std::time::Instant::now() < deadline, "timed out");
                frame(app, vec![]);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        };
        until(&mut app, &|a| a.tabs[0].capabilities.is_some());

        // Without a default ID, Quick Sign explains what it needs instead of guessing.
        app.quick_sign();
        assert!(app.digital.open && app.digital.panel == Panel::Identities);
        assert!(app.digital.error.is_some() && !app.digital.quick);
        app.digital.open = false;
        app.digital.remember(entry.clone());
        app.digital.set_default(Some(&entry.id)).unwrap();
        // Image only needs an image.
        app.digital.layout = AppearanceLayout::ImageOnly;
        app.quick_sign();
        assert!(app.digital.panel == Panel::Appearance && !app.digital.quick);
        app.digital.layout = AppearanceLayout::ImageAndDetails;
        app.digital.open = false;
        app.digital.error = None;

        app.quick_sign();
        assert!(app.digital.quick);
        assert!(
            app.editor.tool == editing::Tool::SignArea && app.editor.mode == editing::Mode::Sign
        );
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        let page = ctx
            .read_response(egui::Id::new((app.tabs[0].id, 0_usize, "page_actions")))
            .unwrap()
            .rect;
        let at = page.min + vec2(300., 400.) * app.tabs[0].scale;
        for pressed in [true, false] {
            frame(
                &mut app,
                vec![
                    egui::Event::PointerMoved(at),
                    egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        // A click places a default box centered on it; a .p12 asks for its passphrase.
        let r = app.digital.request.rect.unwrap();
        assert!((r[2] - r[0] - 240.).abs() < 0.01 && (r[3] - r[1] - 72.).abs() < 0.01);
        assert!(
            ((r[0] + r[2]) / 2. - 300.).abs() < 0.5 && ((r[1] + r[3]) / 2. - 400.).abs() < 0.5,
            "{r:?} page {page:?} scale {}",
            app.tabs[0].scale
        );
        assert!(app.digital.quick_prompt);
        app.digital.sign_password = Zeroizing::new(passphrase.into());
        app.digital.quick_prompt = false;
        app.quick_sign_now(&ctx);
        until(&mut app, &|a| a.tabs.len() == 2 && !a.digital.busy);

        let signed = temp.path().join("Contract-signed 2.pdf");
        assert!(signed.is_file(), "{:?}", app.digital.error);
        assert_eq!(
            std::fs::read(temp.path().join("Contract-signed.pdf")).unwrap(),
            b"keep"
        );
        let report = signing::verify(&std::fs::read(&signed).unwrap(), None).unwrap();
        assert_eq!(report.len(), 1);
        assert!(report[0].crypto_valid && report[0].digest_matches && report[0].byte_range_valid);
        assert!(
            app.status
                .as_ref()
                .is_some_and(|(m, error)| !error && m.contains("Contract-signed 2.pdf"))
        );

        // The chosen appearance is remembered across launches.
        app.digital.layout = AppearanceLayout::ImageOnly;
        app.digital.asset_id = Some("signature-asset".into());
        app.digital.save_appearance();
        let reloaded = DigitalState::at(temp.path().join("profile"));
        assert!(reloaded.layout == AppearanceLayout::ImageOnly);
        assert_eq!(reloaded.asset_id.as_deref(), Some("signature-asset"));
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
