//! Native taskbar Jump List. COM work stays off the rendering thread.
use super::*;
use ::windows::{
    Win32::{
        Foundation::E_ACCESSDENIED,
        Storage::EnhancedStorage::PKEY_Title,
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize, StructuredStorage::PROPVARIANT,
        },
        UI::Shell::{
            Common::{IObjectArray, IObjectCollection},
            DestinationList, EnumerableObjectCollection, ICustomDestinationList, IShellLinkW,
            PropertiesSystem::IPropertyStore,
            ShellLink,
        },
    },
    core::{HSTRING, Interface, Result},
};

#[cfg(not(test))]
pub(super) struct Publisher(std::sync::mpsc::Sender<Snapshot>);
#[cfg(not(test))]
impl Publisher {
    pub(super) fn new() -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<Snapshot>();
        std::thread::spawn(move || {
            // SAFETY: this dedicated thread owns and releases all COM objects.
            if unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_err() {
                return;
            }
            let root = mypdf_milestone0::assets::data_directory();
            let mut removed = preferences::load::<HashSet<String>>(&root, "jump-list-removed.json")
                .unwrap_or_default();
            while let Ok(mut snapshot) = rx.recv() {
                // Coalesce quick tab changes; never rebuild the list per frame.
                while let Ok(newer) = rx.recv_timeout(std::time::Duration::from_millis(200)) {
                    snapshot = newer;
                }
                let before = removed.clone();
                if let Err(error) = update(&snapshot, &mut removed) {
                    eprintln!("Could not update the MyPDF Jump List: {error}");
                }
                if removed != before {
                    if let Err(error) = preferences::save(&root, "jump-list-removed.json", &removed)
                    {
                        eprintln!("Could not remember removed Jump List items: {error}");
                    }
                }
            }
            unsafe {
                CoUninitialize();
            }
        });
        Self(tx)
    }
    pub(super) fn publish(&self, snapshot: Snapshot) {
        let _ = self.0.send(snapshot);
    }
}

fn link(executable: &HSTRING, title: &str, detail: &str, action: &Action) -> Result<IShellLinkW> {
    // SAFETY: all strings remain valid through synchronous COM calls. The
    // interfaces and PROPVARIANT own their references and allocated values.
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(executable)?;
        link.SetArguments(&HSTRING::from(action.arguments()))?;
        link.SetIconLocation(executable, 0)?;
        link.SetDescription(&HSTRING::from(detail))?;
        let properties: IPropertyStore = link.cast()?;
        properties.SetValue(&PKEY_Title, &PROPVARIANT::from(title))?;
        properties.Commit()?;
        Ok(link)
    }
}

fn update(snapshot: &Snapshot, removed: &mut HashSet<String>) -> Result<()> {
    // Use the process's OS-assigned identity: this preserves the packaged
    // Store app's AUMID and the standalone installer's existing taskbar group.
    // SAFETY: called only on a COM-initialized worker; no interface crosses
    // threads. A failed transaction is explicitly aborted.
    unsafe {
        let list: ICustomDestinationList =
            CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER)?;
        let mut slots = 0;
        let excluded: IObjectArray = list.BeginList(&mut slots)?;
        let result = (|| -> Result<()> {
            for index in 0..excluded.GetCount()? {
                if let Ok(link) = excluded.GetAt::<IShellLinkW>(index) {
                    let mut args = vec![0u16; 32_768];
                    link.GetArguments(&mut args)?;
                    let end = args.iter().position(|&c| c == 0).unwrap_or(args.len());
                    let args = String::from_utf16_lossy(&args[..end]);
                    if args.starts_with(ARGUMENT) {
                        removed.insert(args);
                    }
                }
            }
            let executable = HSTRING::from(
                std::env::current_exe()
                    .map_err(|e| {
                        ::windows::core::Error::new(
                            ::windows::core::HRESULT(0x80004005_u32 as i32),
                            e.to_string(),
                        )
                    })?
                    .as_os_str(),
            );
            let mut available = snapshot.clone();
            available
                .recent
                .retain(|entry| matches!(&entry.action, Action::Document(path) if path.is_file()));
            for (category, entries) in jump_categories(&available, slots as usize, removed) {
                let objects: IObjectCollection =
                    CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER)?;
                for entry in entries {
                    objects.AddObject(&link(
                        &executable,
                        &entry.title,
                        &entry.detail,
                        &entry.action,
                    )?)?;
                }
                let array: IObjectArray = objects.cast()?;
                if let Err(error) = list.AppendCategory(&HSTRING::from(category), &array) {
                    // Windows privacy settings may disable recent destinations.
                    // The two non-personal tasks must still remain available.
                    if error.code() == E_ACCESSDENIED {
                        break;
                    }
                    return Err(error);
                }
            }
            let tasks: IObjectCollection =
                CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER)?;
            tasks.AddObject(&link(
                &executable,
                "Open PDF…",
                "Choose a PDF to open",
                &Action::Open,
            )?)?;
            tasks.AddObject(&link(
                &executable,
                "New Blank PDF",
                "Create a blank PDF",
                &Action::New,
            )?)?;
            let tasks: IObjectArray = tasks.cast()?;
            list.AddUserTasks(&tasks)?;
            list.CommitList()
        })();
        if result.is_err() {
            let _ = list.AbortList();
        }
        result
    }
}
