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

mod manager;
use manager::{Look, ManagerState, SavedSignature};

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Panel {
    Signatures,
    Identities,
    Sign,
    Validate,
    Trusted,
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
pub(super) struct DefaultIdentity {
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
    #[serde(default)]
    signatures: Vec<SavedSignature>,
    #[serde(default)]
    default_signature: Option<String>,
    /// Written with saved signatures: the old appearance is not migrated again.
    #[serde(default)]
    signatures_managed: bool,
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
    /// On a smart card or USB token.
    #[serde(default)]
    token: bool,
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
            token: false,
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
            .unwrap_or_else(|| {
                let subject = &self.info.subject;
                subject
                    .split(", ")
                    .find_map(|p| p.strip_prefix("CN="))
                    .unwrap_or(subject)
                    .to_owned()
            })
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
    /// A digital ID created in the system store, for a signature.
    Stored {
        entry: Entry,
        signature: Option<String>,
        /// Whether the system now trusts it (an empty error: not asked to).
        trusted: Result<(), String>,
    },
    /// The operating system's trust in a certificate changed.
    SystemTrust {
        der: Vec<u8>,
        trusted: bool,
    },
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
    report: Option<(u64, u64, Vec<SignatureEvidence>, bool)>,
    trust_id: Option<String>,
    use_explicit_trust: bool,
    tx: mpsc::Sender<Event>,
    rx: mpsc::Receiver<Event>,
    /// Certificates the user trusts for validation, kept between sessions.
    pub(super) trust: mypdf_milestone0::trust::TrustList,
    /// Bumped whenever trust changes, so signature checks run again.
    pub(super) trust_generation: u64,
    /// Whether the operating system trusts a certificate, by DER.
    system_trust: HashMap<Vec<u8>, bool>,
    /// Add a newly created personal ID to the system store and trust it there.
    create_trust: bool,
    /// Saved signatures; the default one drives Quick Sign.
    signatures: Vec<SavedSignature>,
    default_signature: Option<String>,
    manager: ManagerState,
    /// The default signature's look, applied to each signing request.
    look: Look,
    /// Quick Sign into this empty form field: name, page and displayed rectangle.
    pub(super) target_field: Option<(String, usize, [f32; 4])>,
    /// The placed box is a form field's: sign it at exactly that size.
    exact: bool,
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
        let trust_path = root.join("trusted_certificates.json");
        let trust = mypdf_milestone0::trust::TrustList::open(&trust_path)
            .unwrap_or_else(|_| mypdf_milestone0::trust::TrustList::new(&trust_path));
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
            mut signatures,
            mut default_signature,
            signatures_managed,
        } = contents;
        // Earlier versions kept one appearance and a default certificate:
        // they become the first saved signature.
        if !signatures_managed
            && signatures.is_empty()
            && (default_identity.is_some() || appearance.asset_id.is_some())
        {
            let image_only = appearance.layout == AppearanceLayout::ImageOnly;
            let first = SavedSignature {
                identity: default_identity.clone(),
                asset_id: appearance.asset_id.clone(),
                show_name: !image_only,
                show_date: !image_only,
                background: if image_only {
                    signing::Background::Clear
                } else {
                    signing::Background::Framed
                },
                ..Default::default()
            };
            default_signature = Some(first.id.clone());
            signatures.push(first);
        }
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
            panel: Panel::Signatures,
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
            report: None,
            trust_id: None,
            use_explicit_trust: false,
            tx,
            rx,
            trust,
            trust_generation: 0,
            system_trust: HashMap::new(),
            create_trust: cfg!(any(target_os = "macos", target_os = "windows")),
            signatures,
            default_signature,
            manager: ManagerState::new(),
            look: Look::default(),
            target_field: None,
            exact: false,
        };
        state.apply_default_signature();
        state.select_default();
        state
    }
    /// The signature Quick Sign uses.
    fn default_signature(&self) -> Option<&SavedSignature> {
        let id = self.default_signature.as_ref()?;
        self.signatures.iter().find(|s| &s.id == id)
    }
    /// Make the default signature's ID, image and look the ones signing uses.
    pub(super) fn apply_default_signature(&mut self) {
        let Some(signature) = self.default_signature().cloned() else {
            // Every remembered default became a signature when the catalogue was read.
            self.default_identity = None;
            self.look = Look {
                show_name: true,
                show_date: true,
                ..Default::default()
            };
            return;
        };
        self.default_identity = signature.identity.clone();
        self.asset_id = signature.asset_id.clone();
        self.layout = signature.layout().unwrap_or(AppearanceLayout::DetailsOnly);
        self.look = Look::of(&signature);
        self.manager.sign_with = Some(signature.id);
    }
    pub fn use_document(&mut self, tab: Option<&Tab>) {
        if let Some(tab) = tab
            && self.document != Some(tab.id)
        {
            self.document = Some(tab.id);
            self.apply_default_signature();
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
            signatures: self.signatures.clone(),
            default_signature: self.default_signature.clone(),
            signatures_managed: true,
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
                #[cfg(target_os = "macos")]
                token: i.token,
                #[cfg(not(target_os = "macos"))]
                token: false,
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
    Place,
    Sign,
    Verify,
    Forget(String),
    /// Import an image or SVG and use it as the signature artwork.
    ImportArtwork,
    RemoveArtworkBackground(String, mypdf_milestone0::assets::Asset),
    /// Draw or type a signature and use it as the signature artwork.
    DrawArtwork,
    /// Trust a certificate in MyPDF.
    TrustAdd(Vec<u8>),
    /// Stop trusting a certificate in MyPDF (by SHA-256).
    TrustRemove(String),
    /// Read a .cer/.crt/.pem file and trust it in MyPDF.
    TrustImport,
    /// Export a trusted certificate (by SHA-256).
    TrustExport(String),
    /// Trust (or stop trusting) a certificate in the operating system.
    SystemTrust(Vec<u8>, bool),
    /// Open "Create my digital ID", for a signature or for a new one.
    NewId(Option<String>),
    /// Create the digital ID described in the dialog, in the system store.
    CreateInStore,
    /// Start a signature that signs with this digital ID.
    NewSignature(DefaultIdentity),
}

/// The operating system's trust store, at the start of a sentence.
fn system_store_title() -> &'static str {
    if cfg!(target_os = "windows") {
        "Windows"
    } else {
        "This Mac"
    }
}

/// The operating system's trust store, in words for buttons.
fn system_store() -> &'static str {
    if cfg!(target_os = "windows") {
        "Windows"
    } else {
        "this Mac"
    }
}

/// Trust, or stop trusting, a certificate in the operating system.
fn platform_trust(der: &[u8], trust: bool) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    return if trust {
        mypdf_milestone0::keychain::trust(der)
    } else {
        mypdf_milestone0::keychain::untrust(der)
    };
    #[cfg(target_os = "windows")]
    return if trust {
        mypdf_milestone0::windows_trust::trust(der)
    } else {
        mypdf_milestone0::windows_trust::untrust(der)
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (der, trust);
        anyhow::bail!("This system has no certificate trust store MyPDF can change")
    }
}

/// Add a digital ID to the operating system's store (Keychain or Personal store).
fn platform_import(pkcs12: &[u8], password: &str) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    return mypdf_milestone0::keychain::import_identity(pkcs12, password);
    #[cfg(target_os = "windows")]
    return mypdf_milestone0::windows_trust::import_identity(pkcs12, password);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (pkcs12, password);
        anyhow::bail!("This system has no certificate store MyPDF can add to")
    }
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
    pub(super) fn use_transparent_signature_image(
        &mut self,
        id: &str,
        asset: mypdf_milestone0::assets::Asset,
        png: Arc<Vec<u8>>,
    ) {
        let Some(index) = self
            .digital
            .signatures
            .iter()
            .position(|s| s.id == id && s.asset_id.as_ref() == Some(&asset.id))
        else {
            self.status = Some((
                "The signature changed. Select its image again.".into(),
                true,
            ));
            return;
        };
        let next = mypdf_milestone0::assets::Asset::new(
            &format!(
                "Transparent {}",
                asset.name.chars().take(40).collect::<String>()
            ),
            mypdf_milestone0::annotations::Kind::Image { png },
            asset.style,
            asset.aspect,
        );
        let next_id = next.id.clone();
        let result = self
            .editor
            .library
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("The artwork library is unavailable"))
            .and_then(|library| library.put(next));
        match result {
            Ok(()) => {
                self.digital.signatures[index].asset_id = Some(next_id);
                self.digital.commit_signatures();
                self.status = Some((
                    "Transparent signature image saved; the original stays in your library.".into(),
                    false,
                ));
            }
            Err(error) => self.status = Some((error.to_string(), true)),
        }
    }
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
                    let in_system = self.digital.create_trust
                        && cfg!(any(target_os = "macos", target_os = "windows"));
                    if in_system {
                        self.digital.system_trust.insert(entry.der.clone(), true);
                        self.digital.trust_generation += 1;
                    }
                    self.digital.remember(entry);
                    self.digital.creating = false;
                    self.digital.message = if in_system {
                        format!(
                            "Personal ID saved to {} and added to {} as a trusted certificate.",
                            path.display(),
                            system_store_title()
                        )
                    } else {
                        format!(
                            "Personal ID saved to {}. Its self-signed certificate needs separate recipient trust.",
                            path.display()
                        )
                    };
                    if in_system {
                        let path = (!self.digital.keychain_path.trim().is_empty())
                            .then(|| PathBuf::from(self.digital.keychain_path.trim()));
                        self.digital.refresh_store(ctx, path);
                    }
                }
                Event::Exported(path) => {
                    self.digital.message = format!(
                        "Exported {}. Existing files were preserved.",
                        path.display()
                    )
                }
                Event::Stored {
                    entry,
                    signature,
                    trusted,
                } => {
                    let reference = entry.reference();
                    if trusted.is_ok() {
                        self.digital.system_trust.insert(entry.der.clone(), true);
                        self.digital.trust_generation += 1;
                    }
                    self.digital.manager.new_id = None;
                    self.digital.remember(entry);
                    // The new ID signs for the signature that asked for it, or a new one.
                    match signature.filter(|id| self.digital.signatures.iter().any(|s| &s.id == id))
                    {
                        Some(id) => {
                            if let Some(s) = self.digital.signatures.iter_mut().find(|s| s.id == id)
                            {
                                s.identity = Some(reference);
                            }
                            self.digital.manager.selected = Some(id);
                            self.digital.commit_signatures();
                        }
                        None => {
                            self.digital.add_signature(Some(reference));
                        }
                    }
                    self.digital.panel = Panel::Signatures;
                    self.digital.message = match trusted {
                        Ok(()) => format!(
                            "Your digital ID is ready, and {} trusts it.",
                            system_store()
                        ),
                        Err(e) if e.is_empty() => "Your digital ID is ready.".into(),
                        Err(e) => format!(
                            "Your digital ID is ready. It is not marked as trusted on {}: {e}",
                            system_store()
                        ),
                    };
                    #[cfg(not(test))]
                    self.digital.refresh_store(ctx, None);
                }
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
                Event::SystemTrust { der, trusted } => {
                    self.digital.system_trust.insert(der, trusted);
                    self.digital.trust_generation += 1;
                    self.digital.message = if trusted {
                        format!("{} now trusts this certificate.", system_store_title())
                    } else {
                        format!(
                            "{} no longer trusts this certificate.",
                            system_store_title()
                        )
                    };
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
            let target = self
                .digital
                .manager
                .adopt_into
                .take()
                .or_else(|| self.digital.manager.selected.clone());
            match self
                .digital
                .signatures
                .iter_mut()
                .find(|s| Some(&s.id) == target.as_ref())
            {
                Some(signature) => {
                    signature.asset_id = Some(id);
                    self.digital.commit_signatures();
                }
                None => self.digital.asset_id = Some(id),
            }
            self.status = Some(("The image is now part of your signature.".into(), false));
        }
        self.digital.manager.receive(ctx);
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
            let rect = if std::mem::take(&mut self.digital.exact) {
                rect
            } else {
                self.fit_signature_box(tab, page, rect)
            };
            self.digital.document = Some(tab);
            self.digital.request.page = page;
            self.digital.request.rect = Some(rect);
            self.digital.visible = true;
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
        if let Some(action) = self.digital.new_id_dialog(ctx, &mut self.editor) {
            self.digital_action(action, ctx);
        }
        if !self.digital.open {
            self.digital.save_if_dirty();
            return;
        }
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
        egui::Window::new("Signatures")
            .id(egui::Id::new("digital_window"))
            .open(&mut open)
            .collapsible(false)
            .vscroll(true)
            .max_height((ctx.content_rect().height() - 160.0).max(260.0))
            .default_width(640.0)
            .default_height(560.0)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 10.;
                const PANELS: [(Panel, &str); 5] = [
                    (Panel::Signatures, "My signatures"),
                    (Panel::Identities, "Digital IDs"),
                    (Panel::Sign, "Sign"),
                    (Panel::Validate, "Validate"),
                    (Panel::Trusted, "Trusted"),
                ];
                let current = PANELS
                    .iter()
                    .position(|(p, _)| *p == self.digital.panel)
                    .unwrap_or(0);
                if let Some(i) = ui::segmented(ui, &PANELS.map(|(_, name)| name), current) {
                    self.digital.save_if_dirty();
                    self.digital.panel = PANELS[i].0;
                    self.digital.error = None;
                    self.digital.message.clear();
                }
                if self.digital.busy {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("Working… Your files are not changed.").color(theme::muted()));
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
                        ui.label(RichText::new("Looking for digital IDs, cards and tokens…").color(theme::muted()));
                    });
                }
                if let Some(error) = &self.digital.store_error {
                    ui::callout(ui, ui::Tone::Danger, error);
                }
                ui.add_enabled_ui(!self.digital.busy, |ui| match self.digital.panel {
                    Panel::Signatures => {
                        self.digital.signatures_panel(ui, &mut self.editor, &assets, &mut action);
                        if !self.digital.signatures.is_empty() {
                            quick = ui
                                .add_enabled(
                                    active.is_some_and(|t| t.bytes.is_some()),
                                    ActionButton::new(Icon::Sign, "Quick Sign").primary(),
                                )
                                .on_hover_text("Drag a box on the page to sign with the starred signature")
                                .clicked();
                        }
                    }
                    Panel::Identities => {
                        self.digital.identities_panel(ui, &mut self.editor, &mut action);
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
                                #[cfg(any(target_os = "macos", target_os = "windows"))]
                                ui.checkbox(
                                    &mut self.digital.create_trust,
                                    if cfg!(target_os = "windows") {
                                        "Add to Windows and trust it there (Windows asks you to confirm)"
                                    } else {
                                        "Add to Keychain and trust it on this Mac (macOS asks for your password)"
                                    },
                                );
                                ui.label(
                                    RichText::new("A personal ID is self-signed: MyPDF and this computer can trust it, but other people's apps (such as Adobe Acrobat) trust it only after they add your certificate.")
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
                        ui::section(ui, "Signature", |ui| {
                            if self.digital.signatures.is_empty() {
                                ui::callout(
                                    ui,
                                    ui::Tone::Info,
                                    "Create a signature first: choose your digital ID and how it looks.",
                                );
                                if ui.add(ActionButton::new(Icon::Plus, "Create a signature").primary()).clicked() {
                                    self.digital.panel = Panel::Signatures;
                                    self.digital.add_signature(None);
                                }
                                return;
                            }
                            let chosen = self
                                .digital
                                .manager
                                .sign_with
                                .clone()
                                .filter(|id| self.digital.signatures.iter().any(|s| &s.id == id))
                                .or_else(|| self.digital.default_signature.clone())
                                .or_else(|| self.digital.signatures.first().map(|s| s.id.clone()));
                            if let Some(id) = &chosen
                                && self.digital.manager.sign_with.as_ref() != Some(id)
                            {
                                self.digital.use_signature(id);
                            }
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = vec2(10., 10.);
                                for signature in self.digital.signatures.clone() {
                                    let request = self.digital.signature_request(&signature, &assets);
                                    let preview = self.digital.manager.preview(ui.ctx(), &request);
                                    let ready = self.digital.signer_of(&signature).is_some()
                                        && signature.layout().is_some();
                                    if manager::signature_card(
                                        ui,
                                        &signature,
                                        preview.as_ref(),
                                        self.digital.default_signature.as_ref() == Some(&signature.id),
                                        chosen.as_ref() == Some(&signature.id),
                                        ready,
                                    )
                                    .clicked()
                                    {
                                        self.digital.use_signature(&signature.id);
                                    }
                                }
                            });
                            let Some(signature) = chosen
                                .and_then(|id| self.digital.signatures.iter().find(|s| s.id == id))
                                .cloned()
                            else {
                                return;
                            };
                            match self.digital.signer_of(&signature).cloned() {
                                Some(entry) => {
                                    ui.label(
                                        RichText::new(format!(
                                            "Signs with {} · {}",
                                            entry.signer_name(),
                                            manager::place(&entry)
                                        ))
                                        .color(theme::muted()),
                                    );
                                    ui.label(RichText::new(format!("Certificate expires {}",entry.info.not_after)).small().color(theme::muted()));
                                    ui.label(RichText::new(format!("Certificate ID: {}…",entry.info.sha256.chars().take(16).collect::<String>())).small().color(theme::muted())).on_hover_text(&entry.info.sha256);
                                    if matches!(entry.source, Source::File { .. }) {
                                        let r = ui::field(ui, "Passphrase of the digital ID file", |ui| {
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
                                None => {
                                    ui::callout(
                                        ui,
                                        ui::Tone::Warning,
                                        &format!("“{}” has no digital ID that is available now.", signature.name),
                                    );
                                    if ui.add(ActionButton::new(Icon::Pen, "Edit signature")).clicked() {
                                        self.digital.panel = Panel::Signatures;
                                        self.digital.manager.selected = Some(signature.id.clone());
                                    }
                                }
                            }
                        });
                        ui::section(ui, "On the page", |ui| {
                            ui.checkbox(&mut self.digital.visible, "Show the signature on the page");
                            if !self.digital.visible {
                                ui.label(
                                    RichText::new("The document is signed without a visible box. Readers find the signature in their signatures panel.")
                                        .size(12.)
                                        .color(theme::muted()),
                                );
                                return;
                            }
                            let mut request = self.digital.request.clone();
                            request.artwork = self
                                .digital
                                .asset_id
                                .as_ref()
                                .and_then(|id| assets.iter().find(|a| &a.id == id))
                                .cloned();
                            request.layout = self.digital.layout;
                            self.digital.look.apply(&mut request);
                            if let Some([l, t, r, b]) = request.rect {
                                let preview = self.digital.manager.preview(ui.ctx(), &request);
                                match &preview {
                                    Some(Err(e)) => ui::callout(ui, ui::Tone::Warning, e),
                                    _ => {
                                        let (w, h) = (r - l, b - t);
                                        let width = ui.available_width().min(w * 1.4).max(40.);
                                        let (rect, _) = ui.allocate_exact_size(
                                            vec2(width, (width * h / w.max(1.)).min(260.)),
                                            egui::Sense::hover(),
                                        );
                                        ui.painter().rect_stroke(
                                            rect.expand(1.),
                                            6,
                                            Stroke::new(1., theme::line()),
                                            egui::StrokeKind::Outside,
                                        );
                                        manager::paint_preview(
                                            ui,
                                            rect,
                                            preview.as_ref(),
                                            self.digital.look.background == signing::Background::Clear,
                                        );
                                    }
                                }
                            }
                            if let Some(tab) = active {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(format!("Page {}", self.digital.request.page + 1))
                                            .color(theme::muted()),
                                    );
                                    if ui
                                        .add(ActionButton::new(Icon::Rectangle, "Place it again…"))
                                        .on_hover_text("Drag a new box on the page")
                                        .clicked()
                                    {
                                        action = Some(Action::Place);
                                    }
                                    let _ = tab;
                                });
                            }
                        });
                        egui::CollapsingHeader::new("More options").id_salt("sign_more").show(ui, |ui| {
                            let mut focus = false;
                            egui::Grid::new("signature_details")
                                .num_columns(2)
                                .spacing(vec2(12., 8.))
                                .show(ui, |ui| {
                                    for (label, value, hint, limit) in [
                                        ("Reason", &mut self.digital.request.reason, "Optional, such as “I approve this document”", 1000),
                                        ("Location", &mut self.digital.request.location, "Optional, such as Istanbul", 300),
                                        ("Field name", &mut self.digital.request.field, "New signature field", 100),
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
                                    if let Some(tab) = active {
                                        ui.label(RichText::new("Page").color(theme::muted()));
                                        let mut page = self.digital.request.page + 1;
                                        if ui
                                            .add(egui::DragValue::new(&mut page).range(1..=tab.pages.len().max(1)))
                                            .changed()
                                        {
                                            self.digital.request.page = page - 1;
                                        }
                                        ui.end_row();
                                    }
                                });
                            if self.digital.visible
                                && let Some(r) = &mut self.digital.request.rect
                            {
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
                            self.editor.text_editing |= focus;
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
                            RichText::new("This signature covers the document as it is now. Later edits are saved as changes after signing; recipients can see that the document was modified. Signing saves a new copy.")
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
                    Panel::Trusted => {
                        ui.label(
                            RichText::new("Signatures from these certificates show as trusted in MyPDF. You can add a certificate here or from a signature's details.")
                                .color(theme::muted()),
                        );
                        if let Some(system) = mypdf_milestone0::trust::system_name() {
                            ui.label(
                                RichText::new(format!(
                                    "MyPDF also trusts what {system} trusts: its built-in root certificates and any certificate you mark “Always Trust”{}.",
                                    if cfg!(target_os = "macos") { " in Keychain Access" } else { " in your certificate stores" }
                                ))
                                .small()
                                .color(theme::muted()),
                            );
                        }
                        if ui
                            .add(ActionButton::new(Icon::Import, "Add certificate…"))
                            .on_hover_text("Trust a .cer, .crt or .pem certificate")
                            .clicked()
                        {
                            action = Some(Action::TrustImport);
                        }
                        if self.digital.trust.entries().is_empty() {
                            ui::callout(ui, ui::Tone::Info, "No certificates yet. Add one here, or choose “Add to trusted certificates” in a signature's details.");
                        }
                        for t in self.digital.trust.entries() {
                            egui::Frame::new()
                                .fill(theme::surface())
                                .stroke(Stroke::new(1., theme::line()))
                                .corner_radius(10)
                                .inner_margin(12)
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    let name = |dn: &str| {
                                        dn.split(", ")
                                            .find_map(|p| p.strip_prefix("CN="))
                                            .unwrap_or(dn)
                                            .to_owned()
                                    };
                                    let font = ui::strong(ui, 13.5);
                                    ui.label(RichText::new(name(&t.subject)).font(font).color(theme::ink()));
                                    ui.label(
                                        RichText::new(format!("Issued by {} · valid until {}", name(&t.issuer), t.not_after))
                                            .size(12.)
                                            .color(theme::muted()),
                                    );
                                    ui.label(
                                        RichText::new(format!("SHA-256 {}…", &t.sha256[..16.min(t.sha256.len())]))
                                            .size(11.)
                                            .color(theme::muted()),
                                    );
                                    ui.horizontal(|ui| {
                                        if ui.add(ActionButton::new(Icon::Export, "Export…")).clicked() {
                                            action = Some(Action::TrustExport(t.sha256.clone()));
                                        }
                                        if ui.add(ActionButton::new(Icon::Trash, "Remove")).clicked() {
                                            action = Some(Action::TrustRemove(t.sha256.clone()));
                                        }
                                    });
                                });
                        }
                        ui.label(
                            RichText::new("Adobe Acrobat keeps its own list. Someone who receives your signed PDF trusts your certificate in Acrobat from the signature's properties: Show Signer's Certificate, then Trust.")
                                .small()
                                .color(theme::muted()),
                        );
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
        if quick {
            self.quick_sign();
        }
        if !open {
            self.digital.save_if_dirty();
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
        self.digital.look.apply(&mut request);
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
        let target = self.digital.target_field.take();
        if self.signing_problem() {
            return;
        }
        // Quick Sign always uses the starred signature.
        self.digital.save_if_dirty();
        self.digital.apply_default_signature();
        self.digital.select_default();
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
            self.digital.panel = Panel::Signatures;
            self.digital.error = Some(match self.digital.default_signature() {
                None => "Quick Sign needs a signature. Create one: choose your digital ID and how it looks.".into(),
                Some(s) => format!("“{}” needs a digital ID that is available now. Choose one under “Signs with”.", s.name),
            });
            self.digital.manager.selected = self.digital.default_signature.clone();
            return;
        };
        let nothing = self
            .digital
            .default_signature()
            .is_some_and(|s| s.layout().is_none());
        if nothing
            || self.digital.layout == AppearanceLayout::ImageOnly
                && self.signature_artwork().is_none()
        {
            self.digital.open = true;
            self.digital.panel = Panel::Signatures;
            self.digital.manager.selected = self.digital.default_signature.clone();
            self.digital.error = Some(
                "Choose what your signature shows: your name, the date, a note or an image.".into(),
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
        match target {
            // A form's signature field: its name and box, no drawing needed.
            Some((name, page, rect)) => {
                self.digital.request.field = name;
                self.digital.exact = true;
                self.editor.pending_sign_rect = Some((self.tabs[self.active].id, page, rect));
            }
            None => {
                self.editor.mode = editing::Mode::Sign;
                self.editor.start_signature_rectangle();
            }
        }
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
                    let add_to_system = self.digital.create_trust
                        && cfg!(any(target_os = "macos", target_os = "windows"));
                    self.digital.spawn(ctx, move || {
                        let created = identity::create_personal(&details, &password)?;
                        write_new(&path, &created.pkcs12)?;
                        let imported = identity::inspect_pkcs12(&created.pkcs12, &password)?
                            .into_iter()
                            .next()
                            .context("Created identity was not readable")?;
                        if add_to_system {
                            platform_import(&created.pkcs12, &password)?;
                            platform_trust(&imported.certificate_der, true)?;
                        }
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
 let info = identity::info(&der)?; Ok(Event::Certificate(Entry { id: format!("{:032x}", rand::random::<u128>()), source: Source::Certificate, info, der, eligible: false, explanation: Some("A public certificate has no private key. It can be inspected/exported and used for explicit verification, but cannot sign.".into()), token: false })) });
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
            Action::ImportArtwork => {
                self.editor.last_added = None;
                self.digital.adopt_asset = true;
                self.import_library_asset(ctx, None, false);
            }
            Action::RemoveArtworkBackground(signature, asset) => {
                self.remove_signature_image_background(signature, asset);
            }
            Action::DrawArtwork => {
                self.editor.last_added = None;
                self.digital.adopt_asset = true;
                self.editor.signature_open = true;
            }
            Action::TrustAdd(der) => match self.digital.trust.add(&der) {
                Ok(_) => {
                    self.digital.trust_generation += 1;
                    self.digital.message =
                        "MyPDF now trusts this certificate when it checks signatures.".into();
                }
                Err(e) => self.digital.error = Some(format!("{e:#}")),
            },
            Action::TrustRemove(sha256) => match self.digital.trust.remove(&sha256) {
                Ok(_) => {
                    self.digital.trust_generation += 1;
                    self.digital.message = "MyPDF no longer trusts this certificate.".into();
                }
                Err(e) => self.digital.error = Some(format!("{e:#}")),
            },
            Action::TrustImport => {
                if let Some(path) = self.native_pick(
                    rfd::FileDialog::new()
                        .set_title("Trust a certificate")
                        .add_filter("X.509 certificate", &["cer", "crt", "pem", "der"]),
                ) {
                    let added = bounded_read(&path, 1024 * 1024)
                        .and_then(|bytes| mypdf_milestone0::trust::read_certificate(&bytes))
                        .and_then(|der| self.digital.trust.add(&der));
                    match added {
                        Ok(true) => {
                            self.digital.trust_generation += 1;
                            self.digital.message =
                                "Certificate added to your trusted certificates.".into();
                        }
                        Ok(false) => {
                            self.digital.message = "That certificate is already trusted.".into()
                        }
                        Err(e) => self.digital.error = Some(format!("{e:#}")),
                    }
                }
            }
            Action::TrustExport(sha256) => {
                if let Some(trusted) = self
                    .digital
                    .trust
                    .entries()
                    .iter()
                    .find(|t| t.sha256 == sha256)
                    .cloned()
                    && let Some(path) = self.native_save(
                        rfd::FileDialog::new()
                            .set_title("Export certificate • new filename")
                            .add_filter("X.509 certificate", &["cer"])
                            .set_file_name("certificate.cer"),
                    )
                {
                    self.digital.spawn(ctx, move || {
                        write_new(&path, &trusted.der)?;
                        Ok(Event::Exported(path))
                    });
                }
            }
            Action::SystemTrust(der, trust) => {
                self.digital.spawn(ctx, move || {
                    platform_trust(&der, trust)?;
                    let trusted =
                        mypdf_milestone0::trust::system_trusts(std::slice::from_ref(&der));
                    Ok(Event::SystemTrust { der, trusted })
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
            Action::NewId(signature) => {
                if cfg!(any(target_os = "macos", target_os = "windows")) {
                    let name = self
                        .digital
                        .entries
                        .iter()
                        .find(|e| e.eligible)
                        .map(Entry::signer_name)
                        .unwrap_or_default();
                    self.digital.manager.new_id = Some(manager::NewId {
                        signature,
                        name,
                        email: String::new(),
                        trust: true,
                    });
                } else {
                    // Without a system store, the ID is a passphrase-protected file.
                    self.digital.panel = Panel::Identities;
                    self.digital.creating = true;
                }
            }
            Action::CreateInStore => {
                if let Some(draft) = &self.digital.manager.new_id {
                    let (name, email, trust, signature) = (
                        draft.name.clone(),
                        draft.email.clone(),
                        draft.trust,
                        draft.signature.clone(),
                    );
                    self.digital.spawn(ctx, move || {
                        manager::create_in_store(&name, &email, trust, signature)
                    });
                }
            }
            Action::NewSignature(reference) => {
                self.digital.save_if_dirty();
                self.digital.add_signature(Some(reference));
                self.digital.panel = Panel::Signatures;
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
    pub(super) fn preview_trusted(&mut self) {
        for name in ["Jane Appleseed", "Example Studio Signing CA"] {
            let created = identity::create(name, "synthetic-preview-trust", true).unwrap();
            let der = identity::inspect_pkcs12(&created.pkcs12, "synthetic-preview-trust")
                .unwrap()
                .remove(0)
                .certificate_der;
            self.trust.add(&der).unwrap();
        }
        self.open = true;
        self.panel = Panel::Trusted;
    }
    pub(super) fn preview_identities(&mut self, panel: Panel) {
        let make = |name: &str, days: u32| {
            let created = identity::create_personal(
                &PersonalId {
                    name: name.into(),
                    organization: "Example Studio".into(),
                    email: String::new(),
                    validity_days: days,
                    ecdsa: true,
                },
                "synthetic-preview-identity",
            )
            .unwrap();
            (
                identity::info(&created.certificate_der).unwrap(),
                created.certificate_der,
            )
        };
        let keychain = |info: &IdentityInfo| Source::Keychain {
            path: None,
            fingerprint: info.sha256.clone(),
        };
        let samples = [
            ("Jane Appleseed", 730, 0, true, false),
            ("Jane Appleseed (e-signature card)", 20, 0, true, true),
            ("Jane Appleseed (backup)", 365, 1, true, false),
            ("Partner Signing CA", 365, 2, false, false),
        ];
        self.entries = samples
            .into_iter()
            .enumerate()
            .map(|(i, (name, days, kind, eligible, token))| {
                let (info, der) = make(name, days);
                Entry {
                    id: format!("{i:032x}"),
                    source: match kind {
                        0 => keychain(&info),
                        1 => Source::File {
                            path: "/tmp/jane.p12".into(),
                            alias: "jane.p12".into(),
                        },
                        _ => Source::Certificate,
                    },
                    info,
                    der,
                    eligible,
                    explanation: None,
                    token,
                }
            })
            .collect();
        let quick = SavedSignature {
            name: "Work".into(),
            identity: Some(self.entries[0].reference()),
            ..Default::default()
        };
        self.default_signature = Some(quick.id.clone());
        self.signatures = vec![quick];
        self.apply_default_signature();
        self.select_default();
        self.panel = panel;
        self.open = true;
    }
    /// Visual QA: Quick Sign asking for a .p12 passphrase.
    pub(super) fn preview_quick_prompt(&mut self) {
        self.preview_identities(Panel::Sign);
        self.selected = Some(self.entries[2].id.clone());
        self.open = false;
        self.quick_prompt = true;
    }
    /// Visual QA: three saved signatures using library images (newest first).
    pub(super) fn preview_signatures(&mut self, images: &[String]) {
        self.preview_identities(Panel::Signatures);
        let id = Some(self.entries[0].reference());
        let work = SavedSignature {
            name: "Work".into(),
            identity: id.clone(),
            asset_id: images.first().cloned(),
            background: signing::Background::Framed,
            ..Default::default()
        };
        let initials = SavedSignature {
            name: "Initials".into(),
            identity: id.clone(),
            asset_id: images.get(1).cloned(),
            show_name: false,
            show_date: false,
            background: signing::Background::Clear,
            ..Default::default()
        };
        let approval = SavedSignature {
            name: "Approval".into(),
            identity: id,
            note: "Approved".into(),
            background: signing::Background::Tinted,
            ..Default::default()
        };
        self.default_signature = Some(work.id.clone());
        self.manager.selected = Some(work.id.clone());
        self.signatures = vec![work, initials, approval];
        self.apply_default_signature();
        self.message.clear();
    }
    /// Visual QA: "Create my digital ID" on a first run.
    pub(super) fn preview_new_id(&mut self) {
        self.open = true;
        self.panel = Panel::Signatures;
        self.manager.new_id = Some(manager::NewId {
            signature: None,
            name: "Jane Appleseed".into(),
            email: String::new(),
            trust: true,
        });
    }
    /// Whether the previews asked for so far are drawn.
    pub(super) fn previews_ready(&self) -> bool {
        self.manager.idle() && self.manager.drawn() > 0
    }
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
    let badge_width = if default { 82. } else { 0. };
    let elide = |text: &str, font: FontId, color: Color32, width: f32| {
        let mut job = egui::text::LayoutJob::single_section(
            text.to_owned(),
            egui::TextFormat::simple(font, color),
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(40.));
        ui.painter().layout_job(job)
    };
    let title = elide(
        &entry.signer_name(),
        ui::strong(ui, 13.5),
        if entry.eligible {
            theme::ink()
        } else {
            theme::muted()
        },
        right - text_left - badge_width,
    );
    let (detail, tone) = manager::status(entry);
    let detail = match &entry.source {
        Source::File { alias, .. } => format!("{alias} · {detail}"),
        _ => detail,
    };
    let subtitle = elide(
        &detail,
        FontId::proportional(11.5),
        match tone {
            ui::Tone::Danger => theme::danger(),
            ui::Tone::Warning => theme::warning(),
            _ => theme::muted(),
        },
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
            vec2(74., 17.),
        );
        painter.rect_filled(pill, 8., theme::success().gamma_multiply(0.14));
        painter.text(
            pill.center(),
            egui::Align2::CENTER_CENTER,
            "Quick Sign",
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

/// Trust buttons for a certificate: in MyPDF, and in the operating system.
fn trust_controls(
    ui: &mut egui::Ui,
    system: &mut HashMap<Vec<u8>, bool>,
    trust: &mypdf_milestone0::trust::TrustList,
    der: &[u8],
    action: &mut Option<Action>,
) {
    ui.horizontal_wrapped(|ui| {
        if trust.contains(der) {
            if ui
                .add(ActionButton::new(Icon::Close, "Remove from MyPDF trust"))
                .clicked()
            {
                *action = Some(Action::TrustRemove(
                    mypdf_milestone0::identity::fingerprint(der),
                ));
            }
        } else if ui
            .add(ActionButton::new(Icon::Shield, "Trust in MyPDF"))
            .on_hover_text("Show signatures from this certificate as trusted in MyPDF")
            .clicked()
        {
            *action = Some(Action::TrustAdd(der.to_vec()));
        }
        if mypdf_milestone0::trust::system_name().is_some() {
            let trusted = *system
                .entry(der.to_vec())
                .or_insert_with(|| mypdf_milestone0::trust::system_trusts(&[der.to_vec()]));
            let label = if trusted {
                format!("Stop trusting on {}", system_store())
            } else {
                format!("Trust on {}", system_store())
            };
            if ui
                .add(ActionButton::new(Icon::Check, &label))
                .on_hover_text(if cfg!(target_os = "windows") {
                    "Changes your Windows trusted root certificates; Windows asks you to confirm"
                } else {
                    "Changes this certificate's trust in Keychain; macOS asks for your password"
                })
                .clicked()
            {
                *action = Some(Action::SystemTrust(der.to_vec(), !trusted));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transparent_signature_keeps_original_artwork_and_persists_selection() {
        use mypdf_milestone0::annotations::{Kind, Style};
        let ctx = egui::Context::default();
        let temp = tempfile::tempdir().unwrap();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx), vec![]);
        app.digital = DigitalState::at(temp.path().join("digital"));
        app.editor.library =
            Some(mypdf_milestone0::assets::Library::open(temp.path().join("artwork")).unwrap());
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(10, 10, image::Rgba([255, 255, 255, 255]))
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let asset = mypdf_milestone0::assets::Asset::new(
            "Signature",
            Kind::Image {
                png: Arc::new(bytes.into_inner()),
            },
            Style::default(),
            1.,
        );
        app.editor
            .library
            .as_mut()
            .unwrap()
            .put(asset.clone())
            .unwrap();
        let id = app.digital.add_signature(None);
        app.digital.signatures[0].asset_id = Some(asset.id.clone());
        let Kind::Image { png } = &asset.kind else {
            unreachable!()
        };
        let clear =
            Arc::new(mypdf_milestone0::image_background::remove(png, Default::default()).unwrap());
        app.use_transparent_signature_image(&id, asset.clone(), clear);
        let library = app.editor.library.as_ref().unwrap();
        assert_eq!(library.entries.len(), 2);
        assert_eq!(
            library.entries.iter().find(|a| a.id == asset.id),
            Some(&asset)
        );
        let new_id = app.digital.signatures[0].asset_id.clone();
        assert_ne!(new_id, Some(asset.id));
        assert_eq!(
            DigitalState::at(temp.path().join("digital")).signatures[0].asset_id,
            new_id
        );
    }

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
        assert!(state.signatures.is_empty(), "nothing to migrate");
        state.add_signature(Some(state.entries[0].reference()));
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
            token: false,
        };
        let mut state = DigitalState::at(temp.path().into());
        state.reconcile_store(None, vec![entry.clone()]);
        assert!(
            state.selected.is_none(),
            "discovery never silently chooses a certificate"
        );
        let signature = state.add_signature(Some(entry.reference()));
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
        assert!(
            state
                .default_signature()
                .is_some_and(|s| state.signer_of(s).is_none()),
            "the signature waits for its digital ID"
        );
        state.reconcile_store(None, vec![fresh]);
        assert_eq!(state.selected.as_deref(), Some("store-one"));
        // Deleting the last signature does not bring one back on the next launch.
        state.delete_signature(&signature);
        let reloaded = DigitalState::at(temp.path().into());
        assert!(reloaded.default_identity.is_none() && reloaded.signatures.is_empty());
        let mut public = entry;
        public.id = "public-only".into();
        public.source = Source::Certificate;
        public.eligible = false;
        state.remember(public);
        assert!(
            state.signing_entries().all(|e| e.id != "public-only"),
            "a certificate alone cannot sign"
        );
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
    fn certificates_trusted_in_mypdf_change_the_verdict_and_persist() {
        use eframe::App;
        let temp = tempfile::tempdir().unwrap();
        let created = identity::create("Trusted Colleague", "synthetic-trust-flow", true).unwrap();
        let signer =
            identity::FileSigner::load(&created.pkcs12, "synthetic-trust-flow", None).unwrap();
        let der = underskrift::CryptoSigner::certificate_der(&signer).to_vec();
        let font = mypdf_milestone0::document::default_font().unwrap();
        let signed = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(signing::sign_custom(
                &mypdf_milestone0::document::fixture(&font).unwrap(),
                &signer,
                &DesktopSigningRequest::default(),
            ))
            .unwrap();
        let pdf = temp.path().join("Signed.pdf");
        std::fs::write(&pdf, signed).unwrap();
        let ctx = egui::Context::default();
        let mut app = Viewer::new(
            &eframe::CreationContext::_new_kittest(ctx.clone()),
            vec![pdf],
        );
        let profile = temp.path().join("profile");
        app.digital = DigitalState::at(profile.clone());
        let trusted = |app: &Viewer| {
            app.tabs[0]
                .signatures
                .views
                .as_ref()
                .and_then(|v| v.as_ref().ok())
                .and_then(|v| {
                    v.iter()
                        .find_map(|v| v.evidence.as_ref().map(|e| e.trusted_by.clone()))
                })
        };
        let until = |app: &mut Viewer, ready: &dyn Fn(&Viewer) -> bool| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !ready(app) {
                assert!(std::time::Instant::now() < deadline, "{:?}", app.status);
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1100., 760.))),
                        ..Default::default()
                    },
                    |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
                );
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        };
        until(&mut app, &|a| trusted(a).is_some());
        assert_eq!(
            trusted(&app),
            Some(None),
            "a new personal ID is not trusted"
        );

        app.digital_action(Action::TrustAdd(der.clone()), &ctx);
        until(&mut app, &|a| {
            trusted(a) == Some(Some("your trusted certificates".into()))
        });
        // The list is kept for the next launch.
        assert!(DigitalState::at(profile.clone()).trust.contains(&der));

        // The Trusted tab lists it; removing it reverts the verdict.
        app.digital.open = true;
        app.digital.panel = Panel::Trusted;
        until(&mut app, &|a| !a.digital.trust.entries().is_empty());
        let sha = app.digital.trust.entries()[0].sha256.clone();
        app.digital_action(Action::TrustRemove(sha), &ctx);
        until(&mut app, &|a| trusted(a) == Some(None));
        assert!(!DigitalState::at(profile).trust.contains(&der));
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
        assert!(app.digital.open && app.digital.panel == Panel::Signatures);
        assert!(app.digital.error.is_some() && !app.digital.quick);
        app.digital.open = false;
        app.digital.remember(entry.clone());
        let signature = app.digital.add_signature(Some(entry.reference()));
        let edit = |app: &mut Viewer, change: &dyn Fn(&mut SavedSignature)| {
            let saved = app
                .digital
                .signatures
                .iter_mut()
                .find(|s| s.id == signature)
                .unwrap();
            change(saved);
            app.digital.commit_signatures();
        };
        // A signature that shows nothing asks for something to show.
        edit(&mut app, &|s| {
            s.show_name = false;
            s.show_date = false;
        });
        app.quick_sign();
        assert!(app.digital.panel == Panel::Signatures && !app.digital.quick);
        assert!(app.digital.error.as_deref().unwrap().contains("shows"));
        edit(&mut app, &|s| {
            s.show_name = true;
            s.show_date = true;
            s.note = "Approved".into();
            s.background = signing::Background::Tinted;
        });
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
        // Keep the click visible at first-page Fit Width, which is larger
        // than fitting to this fixture's wider, rotated second page.
        let at = page.min + vec2(300., 250.) * app.tabs[0].scale;
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
            ((r[0] + r[2]) / 2. - 300.).abs() < 0.5 && ((r[1] + r[3]) / 2. - 250.).abs() < 0.5,
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

        // The signature's note is the reason, unless one was typed for this signing.
        let doc = lopdf::Document::load_mem(&std::fs::read(&signed).unwrap()).unwrap();
        let reasons: Vec<String> = doc
            .objects
            .values()
            .filter_map(|o| o.as_dict().ok()?.get(b"Reason").ok()?.as_str().ok())
            .map(|r| String::from_utf8_lossy(r).into_owned())
            .collect();
        assert_eq!(reasons, ["Approved"]);
        // Signatures are remembered across launches.
        let reloaded = DigitalState::at(temp.path().join("profile"));
        let saved = reloaded.default_signature().unwrap();
        assert_eq!(saved.note, "Approved");
        assert_eq!(saved.background, signing::Background::Tinted);
        assert_eq!(reloaded.look.note, "Approved");
    }
    #[test]
    fn the_old_appearance_becomes_a_signature_and_signatures_persist() {
        let temp = tempfile::tempdir().unwrap();
        let passphrase = "synthetic-migration-passphrase";
        let created = identity::create("Migrated Signer", passphrase, true).unwrap();
        let file = temp.path().join("migrated.p12");
        std::fs::write(&file, &created.pkcs12).unwrap();
        let entry = Entry::file(
            file,
            identity::inspect_pkcs12(&created.pkcs12, passphrase)
                .unwrap()
                .remove(0),
        );
        // A catalogue written before saved signatures existed.
        std::fs::write(
            temp.path().join("identities.json"),
            serde_json::to_vec(&serde_json::json!({
                "entries": [entry],
                "default_identity": entry.reference(),
                "appearance": { "asset_id": "ink-1", "layout": "ImageOnly" },
            }))
            .unwrap(),
        )
        .unwrap();
        let mut state = DigitalState::at(temp.path().into());
        assert_eq!(state.signatures.len(), 1);
        let first = state.default_signature().unwrap().clone();
        assert_eq!(first.identity, Some(entry.reference()));
        assert_eq!(first.asset_id.as_deref(), Some("ink-1"));
        assert!(!first.show_name && !first.show_date);
        assert_eq!(first.background, signing::Background::Clear);
        assert_eq!(state.layout, AppearanceLayout::ImageOnly);
        assert_eq!(
            state.selected,
            Some(entry.id.clone()),
            "Quick Sign is ready"
        );

        // Rename it; a new signature signs with the same ID and gets its own name.
        state.signatures[0].name = "Handwritten".into();
        let second = state.add_signature(None);
        let added = state.signature(&second).unwrap();
        assert_eq!(added.identity, Some(entry.reference()));
        assert_eq!(added.name, "My signature");
        state.make_default(&second);
        assert!(state.look.show_name && state.layout == AppearanceLayout::DetailsOnly);

        let mut state = DigitalState::at(temp.path().into());
        assert_eq!(state.signatures.len(), 2);
        assert_eq!(state.signatures[0].name, "Handwritten");
        assert_eq!(state.default_signature.as_deref(), Some(second.as_str()));
        // Deleting the Quick Sign signature hands Quick Sign to the next one.
        state.delete_signature(&second);
        assert_eq!(state.default_signature.as_deref(), Some(first.id.as_str()));
        state.delete_signature(&first.id);
        let state = DigitalState::at(temp.path().into());
        assert!(
            state.signatures.is_empty(),
            "the old appearance is not migrated again"
        );
        assert!(state.default_identity.is_none());
    }
    #[test]
    fn a_digital_id_created_in_the_store_signs_for_the_signature_that_asked() {
        let temp = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.digital = DigitalState::at(temp.path().join("profile"));
        let signature = app.digital.add_signature(None);
        assert!(app.digital.signatures[0].identity.is_none());
        // What the creation job reports; the real one adds the ID to Keychain or Windows.
        let created = identity::create("Store Signer", "synthetic-store-signer", true).unwrap();
        let info = identity::info(&created.certificate_der).unwrap();
        let entry = Entry {
            id: "created".into(),
            source: Source::Keychain {
                path: None,
                fingerprint: info.sha256.clone(),
            },
            info,
            der: created.certificate_der,
            eligible: true,
            explanation: None,
            token: false,
        };
        app.digital.busy = true;
        app.digital
            .tx
            .send(Event::Stored {
                entry: entry.clone(),
                signature: Some(signature.clone()),
                trusted: Err(String::new()),
            })
            .unwrap();
        app.digital_events(&ctx);
        assert!(!app.digital.busy);
        assert_eq!(app.digital.signatures.len(), 1);
        assert_eq!(app.digital.signatures[0].identity, Some(entry.reference()));
        assert_eq!(app.digital.default_identity, Some(entry.reference()));
        assert_eq!(app.digital.selected.as_deref(), Some("created"));
        assert!(app.digital.message.contains("ready"));
        assert!(app.digital.panel == Panel::Signatures);
        // With no signature waiting, the new ID gets one of its own.
        let mut other = entry.clone();
        other.id = "second".into();
        other.source = Source::Keychain {
            path: None,
            fingerprint: "second".into(),
        };
        app.digital
            .tx
            .send(Event::Stored {
                entry: other.clone(),
                signature: None,
                trusted: Ok(()),
            })
            .unwrap();
        app.digital_events(&ctx);
        assert_eq!(app.digital.signatures.len(), 2);
        assert_eq!(app.digital.signatures[1].identity, Some(other.reference()));
        assert!(app.digital.system_trust.get(&other.der) == Some(&true));
    }
    #[test]
    fn previews_draw_every_background_with_and_without_an_image() {
        use mypdf_milestone0::annotations::{Kind, Style};
        let ctx = egui::Context::default();
        let temp = tempfile::tempdir().unwrap();
        let mut state = DigitalState::at(temp.path().into());
        let ink = mypdf_milestone0::assets::Asset::new(
            "Ink",
            Kind::Ink {
                strokes: vec![vec![[0.1, 0.6], [0.4, 0.3], [0.7, 0.7], [0.9, 0.4]]],
            },
            Style::default(),
            3.,
        );
        let assets = [ink.clone()];
        let mut keys = HashSet::new();
        for background in signing::Background::CHOICES {
            for (asset, details) in [
                (None, true),
                (Some(ink.id.clone()), true),
                (Some(ink.id.clone()), false),
            ] {
                let signature = SavedSignature {
                    asset_id: asset,
                    show_name: details,
                    show_date: details,
                    background,
                    ..Default::default()
                };
                let request = state.signature_request(&signature, &assets);
                keys.insert(format!("{request:?}"));
                assert!(
                    state.manager.preview(&ctx, &request).is_none(),
                    "drawn in the background"
                );
            }
        }
        assert_eq!(keys.len(), 12);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !state.manager.idle() {
            assert!(std::time::Instant::now() < deadline);
            state.manager.receive(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(state.manager.drawn(), 12);
        assert!(
            state.manager.failures().is_empty(),
            "{:?}",
            state.manager.failures()
        );
        // A missing library image falls back to the details instead of failing.
        let gone = SavedSignature {
            asset_id: Some("deleted".into()),
            ..Default::default()
        };
        assert_eq!(
            state.signature_request(&gone, &assets).layout,
            AppearanceLayout::DetailsOnly
        );
    }
    #[test]
    fn the_signatures_window_creates_edits_and_deletes_signatures() {
        use eframe::App;
        let temp = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = Viewer::new(&eframe::CreationContext::_new_kittest(ctx.clone()), vec![]);
        app.digital = DigitalState::at(temp.path().join("profile"));
        app.digital.open = true;
        let frame = |app: &mut Viewer, events: Vec<egui::Event>| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1100., 900.))),
                    events,
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            )
        };
        // Scroll the window until the control is in view, then click it.
        let click = |app: &mut Viewer, label: &str, in_window: bool| {
            let mut at = None;
            for _ in 0..60 {
                let output = frame(app, vec![]);
                let nodes = output.platform_output.accesskit_update.unwrap().nodes;
                let r = nodes
                    .iter()
                    .find(|(_, n)| n.label() == Some(label))
                    .unwrap_or_else(|| panic!("no “{label}”"))
                    .1
                    .bounds()
                    .unwrap();
                let center = pos2(((r.x0 + r.x1) / 2.) as f32, ((r.y0 + r.y1) / 2.) as f32);
                let window = ctx
                    .memory(|m| m.area_rect(egui::Id::new("digital_window")))
                    .unwrap();
                if !in_window || (window.top() + 40.0..window.bottom() - 16.).contains(&center.y) {
                    at = Some(center);
                    break;
                }
                let delta = if center.y > window.center().y {
                    -60.
                } else {
                    60.
                };
                frame(
                    app,
                    vec![
                        egui::Event::PointerMoved(window.center()),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: vec2(0., delta),
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            let at = at.unwrap_or_else(|| panic!("“{label}” stays out of view"));
            for pressed in [true, false] {
                frame(
                    app,
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
            frame(app, vec![]);
        };
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        // First run: one button creates the first signature, which Quick Sign uses.
        click(&mut app, "Create a signature", true);
        assert_eq!(app.digital.signatures.len(), 1);
        let first = app.digital.signatures[0].id.clone();
        assert_eq!(
            app.digital.default_signature.as_deref(),
            Some(first.as_str())
        );
        click(&mut app, "Soft tint", true);
        assert_eq!(
            app.digital.signatures[0].background,
            signing::Background::Tinted
        );
        click(&mut app, "Date and time", true);
        assert!(!app.digital.signatures[0].show_date);
        click(&mut app, "Duplicate", true);
        assert_eq!(app.digital.signatures.len(), 2);
        assert_eq!(app.digital.signatures[1].name, "My signature copy");
        assert_eq!(
            app.digital.signatures[1].background,
            signing::Background::Tinted
        );
        click(&mut app, "Use for Quick Sign", true);
        let copy = app.digital.signatures[1].id.clone();
        assert_eq!(
            app.digital.default_signature.as_deref(),
            Some(copy.as_str())
        );
        click(&mut app, "Delete…", true);
        assert_eq!(
            app.digital.manager.confirm_delete.as_deref(),
            Some(copy.as_str())
        );
        click(&mut app, "Delete", false);
        assert_eq!(app.digital.signatures.len(), 1);
        assert_eq!(
            app.digital.default_signature.as_deref(),
            Some(first.as_str())
        );
        let reloaded = DigitalState::at(temp.path().join("profile"));
        assert_eq!(reloaded.signatures, app.digital.signatures);
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
