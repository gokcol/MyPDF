//! Native offline Windows translation. Only explicit language-pack downloads use
//! HTTP. Text crosses a C ABI in memory and is never sent to a subprocess/server.
use super::*;
use libloading::Library;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    ffi::{CStr, CString, c_char, c_void},
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

#[derive(Clone, Deserialize)]
struct Pack {
    source: String,
    source_name: String,
    target: String,
    target_name: String,
    version: String,
    url: String,
    sha256: String,
    bytes: u64,
    archive_root: String,
    license: String,
    license_url: String,
}
impl Pack {
    fn key(&self) -> String {
        format!("{}-{}-{}", self.source, self.target, self.version)
    }
}
fn catalog() -> Result<Vec<Pack>, String> {
    let packs: Vec<Pack> =
        serde_json::from_str(include_str!("../../native/translation-ct2/languages.json"))
            .map_err(|_| "The offline language catalog is invalid.")?;
    for pack in &packs {
        let safe = |s: &str| {
            !s.is_empty()
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
        };
        if !safe(&pack.source)
            || !safe(&pack.target)
            || !safe(&pack.version)
            || !safe(&pack.archive_root)
            || !pack.url.starts_with("https://argos-net.com/v1/")
            || pack.sha256.len() != 64
            || !pack.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            || pack.bytes == 0
            || pack.bytes > 512 * 1024 * 1024
            || pack.license.is_empty()
            || !pack.license_url.starts_with("https://")
        {
            return Err("The offline language catalog failed validation.".into());
        }
    }
    Ok(packs)
}
fn pack_for(source: &str, target: &str) -> Result<Option<Pack>, String> {
    Ok(catalog()?
        .into_iter()
        .find(|pack| pack.source == source && pack.target == target))
}
fn model_root() -> PathBuf {
    crate::assets::data_directory().join("translation-models")
}

struct Active {
    sender: mpsc::Sender<Event>,
    cancelled: Arc<AtomicBool>,
}
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static JOBS: OnceLock<Mutex<HashMap<u64, Active>>> = OnceLock::new();
// One loaded model/job at a time bounds model memory and avoids CPU saturation.
static WORKER: Mutex<()> = Mutex::new(());
fn jobs() -> &'static Mutex<HashMap<u64, Active>> {
    JOBS.get_or_init(Mutex::default)
}
fn emit(id: u64, event: Event) {
    let terminal = event.terminal();
    let mut jobs = jobs().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(active) = jobs.get(&id) {
        let _ = active.sender.send(event);
    }
    if terminal {
        jobs.remove(&id);
    }
}
pub(super) fn cancel(id: u64) {
    if let Some(active) = jobs().lock().unwrap_or_else(|e| e.into_inner()).remove(&id) {
        active.cancelled.store(true, Ordering::Release);
        let _ = active.sender.send(Event::Cancelled);
    }
}
pub(super) fn available() -> Result<(), String> {
    Runtime::load().map(|_| ())
}
pub(super) fn start(operation: Operation) -> Result<Job, String> {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let (sender, receiver) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    jobs().lock().unwrap_or_else(|e| e.into_inner()).insert(
        id,
        Active {
            sender,
            cancelled: cancelled.clone(),
        },
    );
    std::thread::Builder::new()
        .name("offline-translation".into())
        .spawn(move || {
            let _guard = WORKER.lock().unwrap_or_else(|e| e.into_inner());
            if cancelled.load(Ordering::Acquire) {
                return;
            }
            let result = run(operation, &model_root(), &cancelled, |event| {
                emit(id, event)
            });
            match result {
                Ok(()) => emit(id, Event::Finished),
                Err(_) if cancelled.load(Ordering::Acquire) => emit(id, Event::Cancelled),
                Err(message) => emit(id, Event::Failed(message)),
            }
        })
        .map_err(|_| {
            cancel(id);
            "Cannot start the offline translation worker.".to_owned()
        })?;
    Ok(Job { id, receiver })
}
fn run(
    operation: Operation,
    root: &Path,
    cancelled: &AtomicBool,
    emit: impl Fn(Event),
) -> Result<(), String> {
    match operation {
        Operation::Languages => {
            let mut names = BTreeMap::new();
            for pack in catalog()? {
                names.insert(pack.source, pack.source_name);
                names.insert(pack.target, pack.target_name);
            }
            emit(Event::Languages(
                names
                    .into_iter()
                    .map(|(id, name)| Language { id, name })
                    .collect(),
            ));
        }
        Operation::Check { source, target } => {
            let state = match pack_for(&source, &target)? {
                None => Availability::Unsupported,
                Some(pack)
                    if installed(root, &pack)
                        && verify_installed(root, &pack, cancelled).is_ok() =>
                {
                    Availability::Installed
                }
                Some(_) => Availability::DownloadRequired,
            };
            emit(Event::Availability(state));
        }
        Operation::Download { source, target } => {
            let pack = pack_for(&source, &target)?
                .ok_or("This offline language pair is not available.")?;
            download(root, &pack, cancelled, &emit)?;
            emit(Event::Availability(Availability::Installed));
        }
        Operation::Translate {
            source,
            target,
            texts,
        } => {
            let pack = pack_for(&source, &target)?
                .ok_or("This offline language pair is not available.")?;
            if !installed(root, &pack) {
                return Err("Download this language pair first. No cloud fallback is used.".into());
            }
            verify_installed(root, &pack, cancelled)?;
            let runtime = Runtime::load()?;
            let engine = runtime.engine(&root.join(pack.key()))?;
            let total = texts.len();
            for (index, input) in texts.iter().enumerate() {
                check_cancelled(cancelled)?;
                let text = engine.translate(input, cancelled)?;
                check_cancelled(cancelled)?;
                emit(Event::Translated { index, text });
                emit(Event::Progress {
                    completed: index + 1,
                    total,
                });
            }
        }
    }
    Ok(())
}
fn check_cancelled(cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::Acquire) {
        Err("Translation cancelled.".into())
    } else {
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
struct Receipt {
    archive_sha256: String,
    files: BTreeMap<String, String>,
}
fn installed(root: &Path, pack: &Pack) -> bool {
    let dir = root.join(pack.key());
    read_receipt(&dir).is_ok_and(|receipt| {
        receipt.archive_sha256 == pack.sha256
            && receipt.files.contains_key("model/model.bin")
            && receipt.files.contains_key("sentencepiece.model")
            && receipt
                .files
                .keys()
                .all(|name| allowed_file(name) && dir.join(name).is_file())
    })
}
fn read_receipt(dir: &Path) -> Result<Receipt, String> {
    let path = dir.join("mypdf-model.json");
    if fs::metadata(&path)
        .map_err(|_| "Language pack is incomplete.")?
        .len()
        > 65536
    {
        return Err("Language pack receipt is invalid.".into());
    }
    serde_json::from_reader(File::open(path).map_err(|_| "Cannot open language pack receipt.")?)
        .map_err(|_| "Language pack receipt is invalid.".into())
}
fn hash_file(path: &Path, cancelled: &AtomicBool) -> Result<String, String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "Cannot read offline language data.")?;
    if !metadata.is_file() || metadata.len() > 1024 * 1024 * 1024 {
        return Err("Offline language data is not a valid bounded file.".into());
    }
    let mut file = File::open(path).map_err(|_| "Cannot read offline language data.")?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        check_cancelled(cancelled)?;
        let count = file
            .read(&mut buffer)
            .map_err(|_| "Cannot read offline language data.")?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn verify_installed(root: &Path, pack: &Pack, cancelled: &AtomicBool) -> Result<(), String> {
    let dir = root.join(pack.key());
    let receipt = read_receipt(&dir)?;
    if receipt.archive_sha256 != pack.sha256 {
        return Err("Language pack version is invalid. Download it again.".into());
    }
    for (name, digest) in receipt.files {
        if !allowed_file(&name) || hash_file(&dir.join(name), cancelled)? != digest {
            return Err("Language pack integrity check failed. Download it again.".into());
        }
    }
    Ok(())
}
fn allowed_file(name: &str) -> bool {
    matches!(
        name,
        "model/model.bin"
            | "model/config.json"
            | "model/shared_vocabulary.txt"
            | "model/shared_vocabulary.json"
            | "model/source_vocabulary.json"
            | "model/target_vocabulary.json"
            | "sentencepiece.model"
            | "metadata.json"
            | "README.md"
    )
}
async fn cancellable<F: std::future::Future>(
    future: F,
    token: &AtomicBool,
) -> Result<F::Output, String> {
    tokio::pin!(future);
    loop {
        check_cancelled(token)?;
        tokio::select! {
            value = &mut future => return Ok(value),
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }
}
fn download(
    root: &Path,
    pack: &Pack,
    cancelled: &AtomicBool,
    emit: &impl Fn(Event),
) -> Result<(), String> {
    fs::create_dir_all(root).map_err(|_| "Cannot create the offline language folder.")?;
    let mut archive = tempfile::NamedTempFile::new_in(root)
        .map_err(|_| "Cannot create the language download.")?;
    // No redirects, credentials, cookies, query strings or document text. The
    // catalog is compiled into this app and every archive has a pinned SHA-256.
    let client = reqwest::Client::builder()
        .https_only(true)
        .http1_only()
        .user_agent(concat!("MyPDF/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|_| "Cannot start the language-pack download.")?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "Cannot start the language download worker.")?;
    let result = runtime.block_on(async {
        let response = cancellable(client.get(&pack.url).send(), cancelled).await?
            .map_err(|_| {
                "The language pack could not be downloaded. Check your connection and try again."
            })?;
        let mut response = response.error_for_status().map_err(
            |_| "The language pack could not be downloaded. Check your connection and try again.",
        )?;
        if response
            .content_length()
            .is_some_and(|size| size != pack.bytes)
        {
            return Err(
                "The language download size changed. Update MyPDF before installing this pack."
                    .to_owned(),
            );
        }
        let mut total = 0u64;
        while let Some(chunk) = cancellable(response.chunk(), cancelled)
            .await?
            .map_err(|_| "The language download was interrupted.")?
        {
            check_cancelled(cancelled)?;
            total += chunk.len() as u64;
            if total > pack.bytes {
                return Err("The language download exceeded its expected size.".into());
            }
            archive
                .write_all(&chunk)
                .map_err(|_| "Cannot save the language download.")?;
            emit(Event::Progress {
                completed: total as usize,
                total: pack.bytes as usize,
            });
        }
        if total != pack.bytes {
            return Err("The language download is incomplete.".into());
        }
        Ok::<(), String>(())
    });
    // A cancelled DNS lookup may be using Tokio's blocking pool. Do not let
    // runtime destruction keep the serialized translation worker waiting for it.
    runtime.shutdown_timeout(Duration::from_millis(100));
    result?;
    install_archive(archive.path(), root, pack, cancelled)
}

fn install_archive(
    archive: &Path,
    root: &Path,
    pack: &Pack,
    cancelled: &AtomicBool,
) -> Result<(), String> {
    if hash_file(archive, cancelled)? != pack.sha256 {
        return Err("The language download failed its integrity check.".into());
    }
    let staging = tempfile::tempdir_in(root).map_err(|_| "Cannot prepare the language pack.")?;
    let file = File::open(archive).map_err(|_| "Cannot open the language download.")?;
    let mut zip =
        zip::ZipArchive::new(file).map_err(|_| "The language download is not a valid package.")?;
    if zip.len() > 1000 {
        return Err("The language package contains too many files.".into());
    }
    let prefix = format!("{}/", pack.archive_root);
    let mut receipt = Receipt {
        archive_sha256: pack.sha256.clone(),
        files: BTreeMap::new(),
    };
    let mut unpacked = 0u64;
    for index in 0..zip.len() {
        check_cancelled(cancelled)?;
        let entry = zip
            .by_index(index)
            .map_err(|_| "Cannot read the language package.")?;
        let Some(name) = entry
            .name()
            .map_err(|_| "Invalid package filename.")?
            .strip_prefix(&prefix)
            .map(str::to_owned)
        else {
            continue;
        };
        if !allowed_file(&name) {
            continue;
        } // Never extract code, Stanza, paths, or symlinks.
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
            || receipt.files.contains_key(&name)
        {
            return Err("The language package has an invalid entry.".into());
        }
        let expected_size = entry.size();
        unpacked = unpacked
            .checked_add(expected_size)
            .ok_or("Language package is too large.")?;
        if unpacked > 1024 * 1024 * 1024 {
            return Err("Language package is too large.".into());
        }
        let path = staging.path().join(&name);
        fs::create_dir_all(path.parent().unwrap()).map_err(|_| "Cannot create language files.")?;
        let mut output = File::create(&path).map_err(|_| "Cannot create a language file.")?;
        let copied = std::io::copy(&mut entry.take(expected_size + 1), &mut output)
            .map_err(|_| "Cannot unpack a language file.")?;
        if copied != expected_size {
            return Err("The language package contains an invalid file size.".into());
        }
        output
            .sync_all()
            .map_err(|_| "Cannot save a language file.")?;
        receipt.files.insert(name, hash_file(&path, cancelled)?);
    }
    if !receipt.files.contains_key("model/model.bin")
        || !receipt.files.contains_key("sentencepiece.model")
    {
        return Err("The language pack is missing its native model or tokenizer.".into());
    }
    let metadata: serde_json::Value = serde_json::from_reader(
        File::open(staging.path().join("metadata.json"))
            .map_err(|_| "The language pack has no identity.")?,
    )
    .map_err(|_| "The language pack identity is invalid.")?;
    if metadata["from_code"] != pack.source || metadata["to_code"] != pack.target {
        return Err("The downloaded language pair does not match the request.".into());
    }
    let json = serde_json::to_vec_pretty(&receipt)
        .map_err(|_| "Cannot record language pack verification.")?;
    fs::write(staging.path().join("mypdf-model.json"), json)
        .map_err(|_| "Cannot record language pack verification.")?;
    fs::write(
        staging.path().join("LICENSE-MyPDF.txt"),
        include_str!("../../native/translation-ct2/licenses/Argos-Translate.txt"),
    )
    .map_err(|_| "Cannot save the language license.")?;
    check_cancelled(cancelled)?;
    let destination = root.join(pack.key());
    // Never remove a working pack before the replacement is fully verified.
    let backup = root.join(format!("{}.previous", pack.key()));
    if backup.exists() {
        fs::remove_dir_all(&backup).map_err(|_| "Cannot prepare language replacement.")?;
    }
    if destination.exists() {
        fs::rename(&destination, &backup)
            .map_err(|_| "Close translation and retry the language update.")?;
    }
    if fs::rename(staging.path(), &destination).is_err() {
        if backup.exists() {
            let _ = fs::rename(&backup, &destination);
        }
        return Err("Cannot install the verified language pack.".into());
    }
    if backup.exists() {
        let _ = fs::remove_dir_all(backup);
    }
    Ok(())
}

type Create = unsafe extern "C" fn(*const c_char, *const c_char) -> *mut c_void;
type Destroy = unsafe extern "C" fn(*mut c_void);
type Translate = unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    usize,
    extern "C" fn(*mut c_void) -> bool,
    *mut c_void,
    *mut *mut c_char,
) -> i32;
type Free = unsafe extern "C" fn(*mut c_char);
struct Runtime {
    _library: Library,
    create: Create,
    destroy: Destroy,
    translate: Translate,
    free: Free,
}
impl Runtime {
    fn load() -> Result<Self, String> {
        let mut directory =
            std::env::current_exe().map_err(|_| "Cannot find the offline translation engine.")?;
        directory.pop();
        #[cfg(target_os = "windows")]
        let path = directory.join("translation/mypdf_ct2.dll");
        #[cfg(not(target_os = "windows"))]
        let path = directory.join("translation/libmypdf_ct2.dylib");
        #[cfg(test)]
        let path = std::env::var_os("MYPDF_CT2_LIBRARY")
            .map(PathBuf::from)
            .unwrap_or(path);
        // Only an absolute app-bundled path is loaded. Never search PATH/CWD.
        if !path.is_absolute() || !path.is_file() {
            return Err("The native offline translation engine is missing. Reinstall the complete MyPDF Windows package. No cloud fallback is used.".into());
        }
        unsafe {
            #[cfg(target_os = "windows")]
            let loaded = libloading::os::windows::Library::load_with_flags(&path, 0x100 | 0x800)
                .map(Library::from);
            #[cfg(not(target_os = "windows"))]
            let loaded = Library::new(&path);
            let library =
                loaded.map_err(|_| "The native offline translation engine could not be loaded.")?;
            let abi: libloading::Symbol<'_, unsafe extern "C" fn() -> u32> = library
                .get(b"mypdf_ct2_abi\0")
                .map_err(|_| "Offline translation engine is incompatible.")?;
            if abi() != 1 {
                return Err("Offline translation engine version is incompatible.".into());
            }
            let create = *library
                .get::<Create>(b"mypdf_ct2_create\0")
                .map_err(|_| "Offline translation engine is incomplete.")?;
            let destroy = *library
                .get::<Destroy>(b"mypdf_ct2_destroy\0")
                .map_err(|_| "Offline translation engine is incomplete.")?;
            let translate = *library
                .get::<Translate>(b"mypdf_ct2_translate\0")
                .map_err(|_| "Offline translation engine is incomplete.")?;
            let free = *library
                .get::<Free>(b"mypdf_ct2_free\0")
                .map_err(|_| "Offline translation engine is incomplete.")?;
            Ok(Self {
                _library: library,
                create,
                destroy,
                translate,
                free,
            })
        }
    }
    fn engine(&self, directory: &Path) -> Result<Engine<'_>, String> {
        let model = CString::new(
            directory
                .join("model")
                .to_str()
                .ok_or("The model path is not valid Unicode.")?,
        )
        .map_err(|_| "The model path is invalid.")?;
        let tokenizer = CString::new(
            directory
                .join("sentencepiece.model")
                .to_str()
                .ok_or("The tokenizer path is not valid Unicode.")?,
        )
        .map_err(|_| "The tokenizer path is invalid.")?;
        let pointer = unsafe { (self.create)(model.as_ptr(), tokenizer.as_ptr()) };
        if pointer.is_null() {
            return Err(
                "The offline language model could not be loaded. Download it again.".into(),
            );
        }
        Ok(Engine {
            runtime: self,
            pointer,
        })
    }
}
struct Engine<'a> {
    runtime: &'a Runtime,
    pointer: *mut c_void,
}
impl Drop for Engine<'_> {
    fn drop(&mut self) {
        unsafe { (self.runtime.destroy)(self.pointer) }
    }
}
extern "C" fn cancelled(context: *mut c_void) -> bool {
    unsafe { (*(context as *const AtomicBool)).load(Ordering::Acquire) }
}
impl Engine<'_> {
    fn translate(&self, text: &str, cancellation: &AtomicBool) -> Result<String, String> {
        // Split on sentence/newline boundaries, then bounded Unicode whitespace.
        // 256 UTF-8 bytes per segment stays well below the native 512-token bound.
        let mut result = String::new();
        for (segment, separator) in segments(text) {
            check_cancelled(cancellation)?;
            if segment.trim().is_empty() {
                result.push_str(segment);
                result.push_str(separator);
                continue;
            }
            let mut output = std::ptr::null_mut();
            let code = unsafe {
                (self.runtime.translate)(
                    self.pointer,
                    segment.as_ptr().cast(),
                    segment.len(),
                    cancelled,
                    (cancellation as *const AtomicBool).cast_mut().cast(),
                    &mut output,
                )
            };
            if code != 0 {
                return Err(match code {
                    1 => "Translation cancelled.",
                    2 => "This text segment is too complex for the offline model. Select a smaller passage.",
                    4 => "The offline model reached its output limit. Select a smaller passage.",
                    _ => "The offline model could not translate this passage.",
                }.into());
            }
            if output.is_null() {
                return Err("The offline model returned no translation.".into());
            }
            let translated = unsafe { CStr::from_ptr(output) }
                .to_str()
                .map(str::to_owned);
            unsafe { (self.runtime.free)(output) };
            result.push_str(&translated.map_err(|_| "The offline model returned invalid text.")?);
            result.push_str(separator);
        }
        Ok(result)
    }
}
fn segments(text: &str) -> Vec<(&str, &str)> {
    let mut result = vec![];
    let mut rest = text;
    while !rest.is_empty() {
        let limit = rest
            .char_indices()
            .find_map(|(i, _)| (i >= 256).then_some(i))
            .unwrap_or(rest.len());
        let candidate = &rest[..limit];
        let end = candidate
            .char_indices()
            .find_map(|(i, ch)| {
                if ch == '\n' {
                    Some(i)
                } else if matches!(ch, '.' | '!' | '?')
                    && rest[i + ch.len_utf8()..].starts_with(char::is_whitespace)
                {
                    Some(i + ch.len_utf8())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| {
                if limit < rest.len() {
                    candidate
                        .char_indices()
                        .rev()
                        .find(|(i, ch)| *i > 0 && ch.is_whitespace())
                        .map_or(limit, |(i, _)| i)
                } else {
                    limit
                }
            });
        let after = &rest[end..];
        let spacing = after.len() - after.trim_start_matches(char::is_whitespace).len();
        result.push((&rest[..end], &after[..spacing]));
        rest = &after[spacing..];
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_has_verified_required_directions_and_additional_languages() {
        let packs = catalog().unwrap();
        for (source, target) in [
            ("en", "tr"),
            ("tr", "en"),
            ("en", "de"),
            ("de", "en"),
            ("en", "fr"),
            ("fr", "en"),
            ("en", "es"),
            ("es", "en"),
        ] {
            assert!(
                packs
                    .iter()
                    .any(|p| p.source == source && p.target == target)
            );
        }
    }
    #[test]
    fn segments_preserve_unicode_and_spacing_without_dropping_content() {
        for input in [
            "İstanbul güzeldir. Straße\n\nHello!  World?",
            &"İıßäöüλ中 ".repeat(100),
            "\n\n",
            "",
        ] {
            let chunks = segments(input);
            assert_eq!(
                chunks
                    .iter()
                    .map(|(a, b)| format!("{a}{b}"))
                    .collect::<String>(),
                input
            );
            assert!(chunks.iter().all(|(a, _)| a.len() <= 260));
        }
    }
    #[test]
    fn corrupt_archive_is_never_installed_and_no_path_escape_is_allowed() {
        let root = tempfile::tempdir().unwrap();
        let archive = root.path().join("bad.zip");
        fs::write(&archive, b"not a model").unwrap();
        let pack = catalog().unwrap().remove(0);
        assert!(install_archive(&archive, root.path(), &pack, &AtomicBool::new(false)).is_err());
        assert!(!root.path().join(pack.key()).exists());
        for name in [
            "../model.bin",
            "model/../../bad",
            "C:/model.bin",
            "stanza/code.py",
            "model/mypdf_ct2.dll",
        ] {
            assert!(!allowed_file(name));
        }
    }
    #[test]
    fn cancellation_discards_late_results() {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = mpsc::channel();
        let token = Arc::new(AtomicBool::new(false));
        jobs().lock().unwrap().insert(
            id,
            Active {
                sender,
                cancelled: token.clone(),
            },
        );
        cancel(id);
        assert!(token.load(Ordering::Acquire));
        assert!(matches!(receiver.recv().unwrap(), Event::Cancelled));
        emit(
            id,
            Event::Translated {
                index: 0,
                text: "private synthetic".into(),
            },
        );
        assert!(receiver.recv().is_err());
    }
    #[test]
    fn cancellation_releases_a_stalled_network_wait_promptly() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let token = AtomicBool::new(false);
        let started = std::time::Instant::now();
        runtime.block_on(async {
            let (result, ()) =
                tokio::join!(cancellable(std::future::pending::<()>(), &token), async {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    token.store(true, Ordering::Release);
                });
            assert!(result.is_err());
        });
        assert!(started.elapsed() < Duration::from_secs(1));
    }
    #[test]
    fn verified_install_ignores_non_model_entries_and_keeps_prior_pack_on_failure() {
        use zip::write::SimpleFileOptions;
        let root = tempfile::tempdir().unwrap();
        let archive = root.path().join("fixture.zip");
        let mut pack = catalog().unwrap().remove(0);
        let mut zip = zip::ZipWriter::new(File::create(&archive).unwrap());
        let metadata = format!(
            r#"{{"from_code":"{}","to_code":"{}"}}"#,
            pack.source, pack.target
        );
        for (name, bytes) in [
            ("model/model.bin", b"synthetic native model".as_slice()),
            ("sentencepiece.model", b"synthetic tokenizer"),
            ("metadata.json", metadata.as_bytes()),
            ("stanza/evil.py", b"not extracted"),
            ("../../escaped.txt", b"not extracted"),
        ] {
            zip.start_file(
                format!("{}/{name}", pack.archive_root),
                SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
        let token = AtomicBool::new(false);
        pack.sha256 = hash_file(&archive, &token).unwrap();
        install_archive(&archive, root.path(), &pack, &token).unwrap();
        assert!(installed(root.path(), &pack));
        verify_installed(root.path(), &pack, &token).unwrap();
        assert!(!root.path().join(pack.key()).join("stanza").exists());
        assert!(!root.path().join("escaped.txt").exists());
        fs::write(&archive, b"incomplete replacement").unwrap();
        assert!(install_archive(&archive, root.path(), &pack, &token).is_err());
        assert!(installed(root.path(), &pack));
        verify_installed(root.path(), &pack, &token).unwrap();
        let cancelled = AtomicBool::new(true);
        assert!(install_archive(&archive, root.path(), &pack, &cancelled).is_err());
        assert!(installed(root.path(), &pack));
        // The public Check action offers repair after an installed file changes.
        let actual = pack_for(&pack.source, &pack.target).unwrap().unwrap();
        let receipt_path = root.path().join(pack.key()).join("mypdf-model.json");
        let mut receipt = read_receipt(&root.path().join(pack.key())).unwrap();
        receipt.archive_sha256 = actual.sha256;
        fs::write(receipt_path, serde_json::to_vec(&receipt).unwrap()).unwrap();
        fs::write(
            root.path().join(pack.key()).join("sentencepiece.model"),
            b"modified",
        )
        .unwrap();
        let state = Mutex::new(None);
        run(
            Operation::Check {
                source: pack.source,
                target: pack.target,
            },
            root.path(),
            &token,
            |event| {
                if let Event::Availability(value) = event {
                    *state.lock().unwrap() = Some(value);
                }
            },
        )
        .unwrap();
        assert_eq!(*state.lock().unwrap(), Some(Availability::DownloadRequired));
    }
    #[test]
    #[ignore = "Explicit network opt-in downloads one approved offline language pack"]
    fn explicit_language_download_verifies_and_installs_the_approved_pack() {
        assert_eq!(std::env::var("MYPDF_CT2_DOWNLOAD_TEST").as_deref(), Ok("1"));
        let root = tempfile::tempdir().unwrap();
        let pack = pack_for("en", "fr").unwrap().unwrap();
        let token = AtomicBool::new(false);
        let progress = Mutex::new(0);
        download(root.path(), &pack, &token, &|event| {
            if let Event::Progress { completed, .. } = event {
                *progress.lock().unwrap() = completed;
            }
        })
        .unwrap();
        assert_eq!(*progress.lock().unwrap(), pack.bytes as usize);
        assert!(installed(root.path(), &pack));
        verify_installed(root.path(), &pack, &token).unwrap();
    }
    #[test]
    #[ignore = "Requires a compiled native bridge and explicitly downloaded test packs"]
    fn native_translation_smoke_all_required_pairs() {
        let downloads = PathBuf::from(
            std::env::var_os("MYPDF_CT2_TEST_PACKS").expect("explicit test pack directory"),
        );
        let root = tempfile::Builder::new()
            .prefix("MyPDF-İstanbul-ß-")
            .tempdir()
            .unwrap();
        let token = AtomicBool::new(false);
        for (source, target, text) in [
            ("en", "tr", "This is a test."),
            ("tr", "en", "Bu bir testtir."),
            ("en", "de", "This is a test."),
            ("de", "en", "Das ist ein Test."),
            ("en", "fr", "This is a test."),
            ("fr", "en", "C'est un test."),
            ("en", "es", "This is a test."),
            ("es", "en", "Esta es una prueba."),
        ] {
            let pack = pack_for(source, target).unwrap().unwrap();
            install_archive(
                &downloads.join(format!("{source}-{target}.argosmodel")),
                root.path(),
                &pack,
                &token,
            )
            .unwrap();
            assert!(installed(root.path(), &pack));
            let output = Mutex::new(String::new());
            run(
                Operation::Translate {
                    source: source.into(),
                    target: target.into(),
                    texts: vec![text.into()],
                },
                root.path(),
                &token,
                |event| {
                    if let Event::Translated { text, .. } = event {
                        *output.lock().unwrap() = text;
                    }
                },
            )
            .unwrap();
            let output = output.into_inner().unwrap();
            assert!(!output.is_empty());
            assert_ne!(output, text);
            let expected = if target == "es" { "prueba" } else { "test" };
            assert!(output.to_lowercase().contains(expected));
            println!("Synthetic {source} → {target}: {output}");
            if source == "en" && target == "tr" {
                let runtime = Runtime::load().unwrap();
                let engine = runtime.engine(&root.path().join(pack.key())).unwrap();
                let cancel = AtomicBool::new(false);
                std::thread::scope(|scope| {
                    scope.spawn(|| {
                        std::thread::sleep(Duration::from_millis(10));
                        cancel.store(true, Ordering::Release);
                    });
                    let started = std::time::Instant::now();
                    assert!(
                        engine
                            .translate(&"This is a long test paragraph. ".repeat(100), &cancel)
                            .is_err()
                    );
                    assert!(
                        started.elapsed() < Duration::from_secs(10),
                        "native token cancellation must be responsive"
                    );
                });
            }
            // Corrupted installed data must fail before entering native inference.
            fs::write(
                root.path().join(pack.key()).join("sentencepiece.model"),
                b"corrupt",
            )
            .unwrap();
            assert!(verify_installed(root.path(), &pack, &token).is_err());
        }
    }
}
