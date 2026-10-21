//! Native taskbar Jump List. COM work stays off the rendering thread.
use super::*;
use ::windows::{
    Win32::{
        Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS},
        Storage::{
            EnhancedStorage::{PKEY_AppUserModel_ID, PKEY_Title},
            Packaging::Appx::GetCurrentApplicationUserModelId,
        },
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize, StructuredStorage::PROPVARIANT,
        },
        UI::Shell::{IShellLinkW, PropertiesSystem::IPropertyStore, ShellLink},
    },
    core::{HSTRING, Interface, PWSTR, Result},
};

#[cfg(not(test))]
use ::windows::Win32::{
    Foundation::E_ACCESSDENIED,
    UI::Shell::{
        Common::{IObjectArray, IObjectCollection},
        DestinationList, EnumerableObjectCollection, ICustomDestinationList,
    },
};

struct LaunchTarget {
    executable: HSTRING,
    icon: HSTRING,
    app_id: Option<HSTRING>,
}
impl LaunchTarget {
    fn current() -> Result<Self> {
        let executable = std::env::current_exe().map_err(|e| {
            ::windows::core::Error::new(
                ::windows::core::HRESULT(0x80004005_u32 as i32),
                e.to_string(),
            )
        })?;
        let mut target = Self {
            executable: HSTRING::from(executable.as_os_str()),
            icon: HSTRING::from(executable.as_os_str()),
            app_id: None,
        };
        let mut length = 0;
        // SAFETY: the first call obtains the buffer size, the second writes
        // at most that many UTF-16 units. Unpackaged processes have no AUMID.
        unsafe {
            if GetCurrentApplicationUserModelId(&mut length, None) == ERROR_INSUFFICIENT_BUFFER
                && (1..=2048).contains(&length)
            {
                let mut buffer = vec![0u16; length as usize];
                if GetCurrentApplicationUserModelId(&mut length, Some(PWSTR(buffer.as_mut_ptr())))
                    == ERROR_SUCCESS
                {
                    let end = buffer.iter().position(|&v| v == 0).unwrap_or(buffer.len());
                    target.app_id = Some(HSTRING::from_wide(&buffer[..end]));
                    // Store installation directories contain a version. Use the
                    // registered alias so pinned links survive an app update.
                    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
                        target.executable = HSTRING::from(
                            PathBuf::from(local)
                                .join("Microsoft/WindowsApps/mypdf-og.exe")
                                .as_os_str(),
                        );
                    }
                }
            }
        }
        Ok(target)
    }
}

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
                if removed != before
                    && let Err(error) = preferences::save(&root, "jump-list-removed.json", &removed)
                {
                    eprintln!("Could not remember removed Jump List items: {error}");
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

fn link(target: &LaunchTarget, title: &str, detail: &str, action: &Action) -> Result<IShellLinkW> {
    // SAFETY: all strings remain valid through synchronous COM calls. The
    // interfaces and PROPVARIANT own their references and allocated values.
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(&target.executable)?;
        link.SetArguments(&HSTRING::from(action.arguments()))?;
        link.SetIconLocation(&target.icon, 0)?;
        link.SetDescription(&HSTRING::from(detail))?;
        let properties: IPropertyStore = link.cast()?;
        properties.SetValue(&PKEY_Title, &PROPVARIANT::from(title))?;
        if let Some(app_id) = &target.app_id {
            properties.SetValue(
                &PKEY_AppUserModel_ID,
                &PROPVARIANT::from(app_id.to_string_lossy().as_str()),
            )?;
        }
        properties.Commit()?;
        Ok(link)
    }
}

#[cfg(not(test))]
fn update(snapshot: &Snapshot, removed: &mut HashSet<String>) -> Result<()> {
    // Use the process's OS-assigned identity: this preserves the packaged
    // Store app's AUMID and the standalone installer's existing taskbar group.
    // SAFETY: called only on a COM-initialized worker; no interface crosses
    // threads. A failed transaction is explicitly aborted.
    unsafe {
        let list: ICustomDestinationList =
            CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER)?;
        let target = LaunchTarget::current()?;
        if let Some(app_id) = &target.app_id {
            list.SetAppID(app_id)?;
        }
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
            let mut available = snapshot.clone();
            available
                .recent
                .retain(|entry| matches!(&entry.action, Action::Document(path) if path.is_file()));
            for (category, entries) in jump_categories(&available, slots as usize, removed) {
                let objects: IObjectCollection =
                    CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER)?;
                for entry in entries {
                    objects.AddObject(&link(
                        &target,
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
                &target,
                "Open PDF…",
                "Choose a PDF to open",
                &Action::Open,
            )?)?;
            tasks.AddObject(&link(
                &target,
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_shell_link_retains_title_and_encoded_activation() {
        // No Jump List is published by this test: it exercises the real Shell
        // property store and link APIs entirely in memory on Windows CI.
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
            let result = (|| -> Result<()> {
                let target = LaunchTarget::current()?;
                let action = Action::Document(PathBuf::from(r"C:\Documents\Örhan work.pdf"));
                let link = link(&target, "Work · Örhan", "PDF shortcut", &action)?;
                let mut args = vec![0u16; 32_768];
                link.GetArguments(&mut args)?;
                let end = args.iter().position(|&c| c == 0).unwrap();
                assert_eq!(String::from_utf16_lossy(&args[..end]), action.arguments());
                let properties: IPropertyStore = link.cast()?;
                assert_eq!(
                    properties.GetValue(&PKEY_Title)?,
                    PROPVARIANT::from("Work · Örhan")
                );
                Ok(())
            })();
            CoUninitialize();
            result.unwrap();
        }
    }
}
