use crate::model::{Config, Profile};
use std::{
    io::{BufRead, BufReader, Read},
    os::windows::{
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    ptr::null_mut,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    System::{Memory::*, Threading::*},
    UI::WindowsAndMessaging::*,
};

pub struct Shared {
    pub config: Config,
    pub active: bool,
    pub status: String,
    pub hotkey_ok: bool,
    pub hovered_profile: Option<(u32, Profile)>,
}
static OWN_CURSOR_CONTEXT: OnceLock<(Arc<Mutex<Shared>>, PathBuf)> = OnceLock::new();
thread_local! {
    static OWN_CURSOR: std::cell::RefCell<Option<(String, u32, HCURSOR)>> = const { std::cell::RefCell::new(None) };
}

unsafe extern "system" fn own_cursor_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && lparam != 0 && (*(lparam as *const CWPRETSTRUCT)).message == WM_SETCURSOR {
        if let Some((shared, bin)) = OWN_CURSOR_CONTEXT.get() {
            if let Ok(state) = shared.try_lock() {
                if let Some(profile) = state.config.app_profile("") {
                    let path = cursor_path(&profile.cursor, bin)
                        .to_string_lossy()
                        .into_owned();
                    OWN_CURSOR.with(|slot| {
                        let mut current = slot.borrow_mut();
                        if current
                            .as_ref()
                            .is_none_or(|(p, s, _)| *p != path || *s != profile.size)
                        {
                            let handle = LoadImageW(
                                null_mut(),
                                wide(&path).as_ptr(),
                                IMAGE_CURSOR,
                                profile.size as i32,
                                profile.size as i32,
                                LR_LOADFROMFILE,
                            ) as HCURSOR;
                            if !handle.is_null() {
                                SetCursor(handle);
                                if let Some((_, _, previous)) =
                                    current.replace((path, profile.size, handle))
                                {
                                    DestroyCursor(previous);
                                }
                            }
                        } else if let Some((_, _, handle)) = current.as_ref() {
                            SetCursor(*handle);
                        }
                    });
                }
            }
        }
    }
    CallNextHookEx(null_mut(), code, wparam, lparam)
}
#[derive(Clone)]
struct Target {
    pid: u32,
    tid: u32,
    path: String,
    x86: bool,
    hwnd: isize,
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn target(hwnd: HWND) -> Option<Target> {
    unsafe {
        if hwnd.is_null() {
            return None;
        }
        let mut pid = 0;
        let tid = GetWindowThreadProcessId(hwnd, &mut pid);
        if tid == 0 {
            return None;
        }
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut buf = vec![0u16; 32768];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut len) != 0;
        let mut wow = 0;
        let arch_ok = IsWow64Process(handle, &mut wow) != 0;
        CloseHandle(handle);
        if !ok || !arch_ok {
            return None;
        }
        Some(Target {
            pid,
            tid,
            path: String::from_utf16_lossy(&buf[..len as usize]),
            x86: wow != 0,
            hwnd: hwnd as isize,
        })
    }
}
pub fn foreground_path() -> Option<String> {
    unsafe {
        let target = target(GetForegroundWindow())?;
        (target.pid != std::process::id()).then_some(target.path)
    }
}
fn hovered() -> Option<Target> {
    unsafe {
        let mut point = POINT { x: 0, y: 0 };
        (GetCursorPos(&mut point) != 0)
            .then(|| target(WindowFromPoint(point)))
            .flatten()
    }
}

fn refresh_cursor(target: &Target) {
    unsafe {
        let mut point = POINT { x: 0, y: 0 };
        if GetCursorPos(&mut point) == 0 {
            return;
        }
        let hovered = WindowFromPoint(point);
        let mut hovered_pid = 0;
        if hovered.is_null()
            || GetWindowThreadProcessId(hovered, &mut hovered_pid) != target.tid
            || hovered_pid != target.pid
        {
            return;
        }
        PostMessageW(
            hovered,
            WM_SETCURSOR,
            hovered as usize,
            (HTCLIENT as isize) | ((WM_MOUSEMOVE as isize) << 16),
        );
    }
}

pub fn cursor_path(cursor: &str, bin: &Path) -> PathBuf {
    if let Some(relative) = cursor.strip_prefix("@yolo\\") {
        bin.join("Cursors").join(relative)
    } else {
        PathBuf::from(cursor)
    }
}

pub fn validate_cursor(path: &Path) -> Result<(), String> {
    unsafe {
        if !matches!(
            path.extension()
                .and_then(|x| x.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref(),
            Some("cur" | "ani")
        ) {
            return Err("Choose a .cur or .ani file".into());
        }
        if !path.is_file() {
            return Err("Cursor file does not exist".into());
        }
        let h = LoadImageW(
            null_mut(),
            wide(&path.to_string_lossy()).as_ptr(),
            IMAGE_CURSOR,
            0,
            0,
            LR_LOADFROMFILE,
        ) as HCURSOR;
        if h.is_null() {
            return Err("Windows could not load this cursor".into());
        }
        DestroyCursor(h);
        Ok(())
    }
}

struct Session {
    target: Target,
    profile: Profile,
    show_hidden: bool,
    child: Child,
    mapping: Option<OwnedHandle>,
}
struct SystemCursor {
    child: Child,
    backup: HCURSOR,
    cursor: String,
    size: u32,
}
impl Drop for SystemCursor {
    fn drop(&mut self) {
        self.child.stdin.take();
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let restored = self.child.wait().is_ok_and(|status| status.success());
        unsafe {
            if restored {
                DestroyCursor(self.backup);
            } else {
                // The watchdog normally restores this on pipe closure. If the
                // watchdog itself failed, the controller owns a second copy.
                SetSystemCursor(self.backup, 32512);
            }
        }
    }
}

fn hook_directory(bin: &Path) -> PathBuf {
    std::env::var_os("MOUSEUP_HOOK_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| bin.to_path_buf())
}

fn system_cursor(cursor: &str, size: u32, bin: &Path) -> Result<SystemCursor, String> {
    let path = cursor_path(cursor, bin);
    validate_cursor(&path)?;
    let backup = unsafe { CopyIcon(LoadCursorW(null_mut(), IDC_ARROW)) as HCURSOR };
    if backup.is_null() {
        return Err("Could not save the system cursor".into());
    }
    let mut child = match Command::new(hook_directory(bin).join("mouseup-hook-host64.exe"))
        .arg("--system-cursor")
        .arg(path)
        .arg(size.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(0x08000000)
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            unsafe {
                DestroyCursor(backup);
            }
            return Err(format!("Could not start the system cursor helper: {error}"));
        }
    };
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let mut line = String::new();
        let _ = BufReader::new(stdout).read_line(&mut line);
        let _ = tx.send(line);
    });
    if !rx
        .recv_timeout(Duration::from_secs(5))
        .is_ok_and(|line| line.trim() == "READY")
    {
        let _ = child.kill();
        let _ = child.wait();
        unsafe {
            SetSystemCursor(backup, 32512);
        }
        return Err("Could not activate the system cursor".into());
    }
    Ok(SystemCursor {
        child,
        backup,
        cursor: cursor.into(),
        size,
    })
}
impl Drop for Session {
    fn drop(&mut self) {
        drop(self.mapping.take());
        self.child.stdin.take();
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        refresh_cursor(&self.target);
    }
}
fn attach(
    target: Target,
    profile: Profile,
    bin: &Path,
    show_hidden: bool,
) -> Result<Session, String> {
    let cursor = cursor_path(&profile.cursor, bin);
    validate_cursor(&cursor)?;
    if cursor.to_string_lossy().contains(['\n', '\r']) {
        return Err("Cursor path contains a newline".into());
    }
    let arch = if target.x86 { "32" } else { "64" };
    // An explicit override is useful for testing a previously registered hook
    // path when Windows refuses to register another DLL during this session.
    let hook_dir = hook_directory(bin);
    let host = hook_dir.join(format!("mouseup-hook-host{arch}.exe"));
    let dll = hook_dir.join(format!("mouseup-hook{arch}.dll"));
    if !host.is_file() || !dll.is_file() {
        return Err(format!(
            "Missing {arch}-bit hook components; use the packaged build"
        ));
    }
    let spec = format!(
        "{}\n{}\n{}\n",
        profile.size,
        cursor.to_string_lossy(),
        u8::from(show_hidden)
    );
    if spec.len() >= 65536 || spec.contains('\0') {
        return Err("Cursor path is too long or invalid".into());
    }
    let name = wide(&format!(
        "Local\\MouseUp-cursor-{}-{}",
        target.pid, target.tid
    ));
    let mapping = unsafe {
        let raw = CreateFileMappingW(
            INVALID_HANDLE_VALUE,
            null_mut(),
            PAGE_READWRITE,
            0,
            65536,
            name.as_ptr(),
        );
        if raw.is_null() {
            return Err(format!(
                "Could not create cursor settings: {}",
                std::io::Error::last_os_error()
            ));
        }
        let mapping = OwnedHandle::from_raw_handle(raw);
        let view = MapViewOfFile(mapping.as_raw_handle(), FILE_MAP_WRITE, 0, 0, 65536);
        if view.Value.is_null() {
            return Err(format!(
                "Could not map cursor settings: {}",
                std::io::Error::last_os_error()
            ));
        }
        std::ptr::copy_nonoverlapping(spec.as_ptr(), view.Value.cast::<u8>(), spec.len());
        *view.Value.cast::<u8>().add(spec.len()) = 0;
        UnmapViewOfFile(view);
        mapping
    };
    let result = (|| -> Result<Child, String> {
        let mut child = Command::new(host)
            .args([
                target.tid.to_string(),
                dll.to_string_lossy().into_owned(),
                target.hwnd.to_string(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags(0x08000000)
            .spawn()
            .map_err(|e| e.to_string())?;
        if let Some(stdout) = child.stdout.take() {
            let (tx, rx) = std::sync::mpsc::channel();
            thread::spawn(move || {
                let mut line = String::new();
                let _ = BufReader::new(stdout).read_line(&mut line);
                let _ = tx.send(line);
            });
            if rx
                .recv_timeout(Duration::from_secs(5))
                .is_ok_and(|line| line.trim() == "READY")
            {
                return Ok(child);
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        let mut detail = String::new();
        if let Some(mut stderr) = child.stderr.take() {
            let _ = stderr.read_to_string(&mut detail);
        }
        if detail.contains("os error 126") {
            Err("Windows could not register the cursor hook (126). Try restarting Windows.".into())
        } else {
            Err(format!("Hook could not be installed: {}", detail.trim()))
        }
    })();
    match result {
        Ok(child) => Ok(Session {
            target,
            profile,
            show_hidden,
            child,
            mapping: Some(mapping),
        }),
        Err(e) => Err(e),
    }
}
pub fn start(
    shared: Arc<Mutex<Shared>>,
    alive: Arc<AtomicBool>,
    bin: PathBuf,
    config_path: PathBuf,
) {
    let _ = OWN_CURSOR_CONTEXT.set((shared.clone(), bin.clone()));
    thread::spawn(move || {
        let mut session: Option<Session> = None;
        let mut system: Option<SystemCursor> = None;
        let mut own_hook: Option<(Target, Profile, HHOOK)> = None;
        let mut retiring: Vec<(u32, thread::JoinHandle<()>)> = Vec::new();
        let mut last_failure = Instant::now() - Duration::from_secs(10);
        let mut last_system_failure = Instant::now() - Duration::from_secs(10);
        let mut last_target = None;
        while alive.load(Ordering::Relaxed) {
            let t = hovered();
            let (enabled, profile, show_hidden, preferred, size, target) = {
                let s = shared.lock().unwrap();
                let p = t.as_ref().and_then(|t| s.config.app_profile(&t.path));
                (
                    s.config.enabled,
                    p,
                    s.config.software_overlay,
                    s.config.preferred_cursor.clone(),
                    s.config.cursor_size,
                    t,
                )
            };
            let system_enabled =
                enabled && !preferred.is_empty() && (target.is_none() || profile.is_some());
            if system
                .as_ref()
                .is_some_and(|s| !system_enabled || s.cursor != preferred || s.size != size)
            {
                system = None;
            }
            if let Some(s) = system.as_mut() {
                if s.child.try_wait().ok().flatten().is_some() {
                    system = None;
                    last_system_failure = Instant::now();
                }
            }
            if system_enabled
                && system.is_none()
                && last_system_failure.elapsed() > Duration::from_secs(3)
            {
                match system_cursor(&preferred, size, &bin) {
                    Ok(helper) => system = Some(helper),
                    Err(error) => {
                        last_system_failure = Instant::now();
                        shared.lock().unwrap().status = error;
                    }
                }
            }
            let target_id = target.as_ref().map(|t| (t.pid, t.tid));
            if target_id != last_target {
                last_failure = Instant::now() - Duration::from_secs(10);
                last_target = target_id;
            }
            let matched = enabled && profile.is_some() && target.is_some();
            retiring.retain(|(_, worker)| !worker.is_finished());
            let own_target =
                matched && target.as_ref().is_some_and(|t| t.pid == std::process::id());
            if own_hook.as_ref().is_some_and(|(old, _, _)| {
                !own_target || target.as_ref().is_none_or(|t| t.tid != old.tid)
            }) {
                let (old, _, hook) = own_hook.take().unwrap();
                unsafe {
                    UnhookWindowsHookEx(hook);
                }
                refresh_cursor(&old);
            }
            if own_target {
                let target = target.as_ref().unwrap();
                let profile = profile.as_ref().unwrap();
                if let Some((old, current, _)) = own_hook.as_mut() {
                    *old = target.clone();
                    if current != profile {
                        *current = profile.clone();
                        refresh_cursor(target);
                    }
                } else {
                    let hook = unsafe {
                        SetWindowsHookExW(
                            WH_CALLWNDPROCRET,
                            Some(own_cursor_hook),
                            null_mut(),
                            target.tid,
                        )
                    };
                    if !hook.is_null() {
                        own_hook = Some((target.clone(), profile.clone(), hook));
                        refresh_cursor(target);
                    }
                }
            }
            if session.as_ref().is_some_and(|s| {
                !matched
                    || target.as_ref().is_none_or(|t| {
                        t.pid != s.target.pid
                            || t.tid != s.target.tid
                            || t.path != s.target.path
                            || t.x86 != s.target.x86
                    })
                    || profile.as_ref() != Some(&s.profile)
                    || show_hidden != s.show_hidden
            }) {
                let old = session.take().unwrap();
                if target.as_ref().is_some_and(|t| t.pid != old.target.pid) {
                    let pid = old.target.pid;
                    retiring.push((pid, thread::spawn(move || drop(old))));
                } else {
                    drop(old);
                }
            }
            if let Some(s) = session.as_mut() {
                if s.child.try_wait().ok().flatten().is_some() {
                    session = None;
                    last_failure = Instant::now();
                }
            }
            if matched
                && !own_target
                && session.is_none()
                && target
                    .as_ref()
                    .is_some_and(|t| !retiring.iter().any(|(pid, _)| *pid == t.pid))
                && last_failure.elapsed() > Duration::from_secs(3)
            {
                match attach(target.unwrap(), profile.unwrap(), &bin, show_hidden) {
                    Ok(s) => {
                        refresh_cursor(&s.target);
                        if s.target.pid != std::process::id() {
                            let mut state = shared.lock().unwrap();
                            if !state
                                .config
                                .profiles
                                .iter()
                                .any(|p| p.executable.eq_ignore_ascii_case(&s.target.path))
                            {
                                state.config.set_app_cursor(&s.target.path, "@global");
                                if let Err(e) = crate::model::save(&config_path, &state.config) {
                                    state.status = format!("Could not save applications: {e}");
                                }
                            }
                        }
                        session = Some(s);
                        shared.lock().unwrap().status = "Cursor active".into();
                    }
                    Err(e) => {
                        last_failure = Instant::now();
                        shared.lock().unwrap().status = e;
                    }
                }
            }
            {
                let mut s = shared.lock().unwrap();
                s.active = session.is_some() || own_hook.is_some();
                // Do not draw the software cursor when hook installation failed or
                // while switching targets: it would appear over the normal cursor.
                s.hovered_profile = session
                    .as_ref()
                    .map(|session| (session.target.pid, session.profile.clone()))
                    .or_else(|| {
                        own_hook
                            .as_ref()
                            .map(|(target, profile, _)| (target.pid, profile.clone()))
                    });
                if own_hook.is_some() {
                    s.status = "Cursor active".into();
                }
                if !enabled {
                    s.status = "Paused".into();
                } else if !matched {
                    s.status = "Original cursor".into();
                }
            }
            thread::sleep(Duration::from_millis(50));
        }
        drop(session);
        drop(system);
        for (_, worker) in retiring {
            let _ = worker.join();
        }
        if let Some((_, _, hook)) = own_hook.take() {
            unsafe {
                UnhookWindowsHookEx(hook);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    #[ignore = "temporarily changes the current desktop's normal system cursor"]
    fn system_cursor_restores_after_helper_exit_and_failure() {
        unsafe fn snapshot() -> Vec<u32> {
            crate::overlay::render(LoadCursorW(null_mut(), IDC_ARROW), 64).unwrap()
        }
        let bin = PathBuf::from(std::env::var_os("MOUSEUP_TEST_PACKAGE").unwrap());
        let before = unsafe { snapshot() };
        for fail in [false, true] {
            let mut system = system_cursor("@yolo\\Arrow\\Blue.cur", 64, &bin).unwrap();
            assert_ne!(unsafe { snapshot() }, before);
            if fail {
                system.child.kill().unwrap();
            }
            drop(system);
            assert_eq!(unsafe { snapshot() }, before);
        }
    }

    #[test]
    #[ignore = "temporarily changes the desktop cursor and terminates a test controller"]
    fn system_cursor_restores_after_controller_crash() {
        struct Restore(HCURSOR);
        impl Drop for Restore {
            fn drop(&mut self) {
                unsafe {
                    SetSystemCursor(self.0, 32512);
                }
            }
        }
        unsafe fn snapshot() -> Vec<u32> {
            crate::overlay::render(LoadCursorW(null_mut(), IDC_ARROW), 64).unwrap()
        }
        let bin = PathBuf::from(std::env::var_os("MOUSEUP_TEST_PACKAGE").unwrap());
        let restore = Restore(unsafe { CopyIcon(LoadCursorW(null_mut(), IDC_ARROW)) as HCURSOR });
        assert!(!restore.0.is_null());
        let before = unsafe { snapshot() };
        let mut controller = Command::new("python")
            .args(["-c", "import subprocess,sys; p=subprocess.Popen([sys.argv[1], '--system-cursor', sys.argv[2], '64'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,creationflags=0x08000000); print(p.stdout.readline().decode().strip(),flush=True); sys.stdin.buffer.read()"])
            .arg(bin.join("mouseup-hook-host64.exe"))
            .arg(cursor_path("@yolo\\Arrow\\Blue.cur", &bin))
            .stdin(Stdio::piped()).stdout(Stdio::piped())
            .creation_flags(0x08000000).spawn().unwrap();
        let mut ready = String::new();
        BufReader::new(controller.stdout.take().unwrap())
            .read_line(&mut ready)
            .unwrap();
        assert_eq!(ready.trim(), "READY");
        assert_ne!(unsafe { snapshot() }, before);
        controller.kill().unwrap();
        controller.wait().unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while unsafe { snapshot() } != before && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(unsafe { snapshot() }, before);
        drop(restore);
    }

    #[test]
    #[ignore = "requires locally extracted cursor assets and a Windows desktop"]
    fn own_window_cursor_changes_without_external_dll() {
        unsafe {
            let bin = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/yolomouse");
            let mut config = Config::default();
            config.library = vec![
                "@yolo\\Arrow\\Blue.cur".into(),
                "@yolo\\Circle\\Pink.cur".into(),
            ];
            config.select_cursor(&config.library[0].clone());
            let shared = Arc::new(Mutex::new(Shared {
                config,
                active: false,
                status: String::new(),
                hotkey_ok: true,
                hovered_profile: None,
            }));
            assert!(OWN_CURSOR_CONTEXT.set((shared.clone(), bin)).is_ok());
            let name = wide("STATIC");
            let hwnd = CreateWindowExW(
                0,
                name.as_ptr(),
                name.as_ptr(),
                WS_OVERLAPPED,
                0,
                0,
                100,
                100,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
            );
            assert!(!hwnd.is_null());
            let hook = SetWindowsHookExW(
                WH_CALLWNDPROCRET,
                Some(own_cursor_hook),
                null_mut(),
                GetCurrentThreadId(),
            );
            assert!(!hook.is_null());
            let arrow = LoadCursorW(null_mut(), IDC_ARROW);
            SetCursor(arrow);
            SendMessageW(hwnd, WM_SETCURSOR, hwnd as usize, HTCLIENT as isize);
            let first = GetCursor();
            assert!(!first.is_null() && first != arrow);
            shared
                .lock()
                .unwrap()
                .config
                .select_cursor("@yolo\\Circle\\Pink.cur");
            SendMessageW(hwnd, WM_SETCURSOR, hwnd as usize, HTCLIENT as isize);
            let second = GetCursor();
            assert!(!second.is_null() && second != first && second != arrow);
            UnhookWindowsHookEx(hook);
            SetCursor(arrow);
            DestroyWindow(hwnd);
        }
    }

    #[test]
    #[ignore = "requires packaged components and both compiled hook_probe examples"]
    fn packaged_backend_attaches_and_restores_both_architectures() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let temp =
            std::env::temp_dir().join(format!("mouseup-backend-test-{}", std::process::id()));
        fs::create_dir_all(&temp).unwrap();
        let bin = if std::env::var_os("MOUSEUP_TEST_BUNDLE").is_some() {
            crate::bundle::prepare(&temp)
                .unwrap()
                .expect("test binary has no bundle")
        } else {
            std::env::var_os("MOUSEUP_TEST_PACKAGE")
                .map(PathBuf::from)
                .unwrap_or_else(|| root.join("dist-next"))
        };
        let previous = std::env::var_os("APPDATA");
        std::env::set_var("APPDATA", &temp);
        for x86 in [false, true] {
            let result = temp.join(if x86 { "x86.txt" } else { "x64.txt" });
            let exe = if x86 {
                root.join("target/i686-pc-windows-msvc/release/examples/hook_probe.exe")
            } else {
                root.join("target/release/examples/hook_probe.exe")
            };
            let mut probe = Command::new(exe)
                .arg(&result)
                .env("APPDATA", temp.join("different-appdata"))
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            let info = loop {
                if let Ok(s) = fs::read_to_string(&result) {
                    let parts: Vec<_> = s.split_whitespace().collect();
                    if parts.len() == 4 {
                        break (
                            parts[0].parse::<u32>().unwrap(),
                            parts[1].parse::<u32>().unwrap(),
                            parts[2].parse::<isize>().unwrap(),
                        );
                    }
                }
                assert!(Instant::now() < deadline, "probe did not start");
                thread::sleep(Duration::from_millis(20));
            };
            let target = Target {
                pid: info.0,
                tid: info.1,
                path: String::new(),
                x86,
                hwnd: info.2,
            };
            for (cursor, show_hidden) in [
                ("@yolo\\Arrow\\Blue.cur", true),
                ("@yolo\\Circle\\Pink.cur", false),
                ("@yolo\\Circle\\Pink.cur", true),
            ] {
                fs::write(
                    result.with_extension("show-hidden"),
                    if show_hidden { "1" } else { "0" },
                )
                .unwrap();
                let profile = Profile {
                    executable: String::new(),
                    cursor: cursor.into(),
                    size: 64,
                };
                let mut session = attach(target.clone(), profile, &bin, show_hidden)
                    .unwrap_or_else(|e| {
                        panic!(
                            "attach x86={x86} cursor={cursor} probe={:?}: {e}",
                            probe.try_wait()
                        )
                    });
                let deadline = Instant::now() + Duration::from_secs(5);
                while !fs::read_to_string(&result)
                    .unwrap_or_default()
                    .ends_with("replaced")
                {
                    assert!(Instant::now() < deadline, "cursor not replaced");
                    thread::sleep(Duration::from_millis(20));
                }
                // Closing the controller pipe also models abrupt controller exit; the
                // helper must detach the detour before removing its message hook.
                drop(session.mapping.take());
                drop(session.child.stdin.take());
                let deadline = Instant::now() + Duration::from_secs(5);
                while session.child.try_wait().unwrap().is_none() {
                    assert!(Instant::now() < deadline, "hook host did not exit");
                    thread::sleep(Duration::from_millis(20));
                }
                let deadline = Instant::now() + Duration::from_secs(5);
                while !fs::read_to_string(&result)
                    .unwrap_or_default()
                    .ends_with("original")
                {
                    assert!(Instant::now() < deadline, "cursor not restored");
                    thread::sleep(Duration::from_millis(20));
                }
                drop(session);
            }
            let profile = Profile {
                executable: String::new(),
                cursor: "@yolo\\Arrow\\Blue.cur".into(),
                size: 64,
            };
            let mut session = attach(target.clone(), profile, &bin, true).unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while !fs::read_to_string(&result)
                .unwrap_or_default()
                .ends_with("replaced")
            {
                assert!(
                    Instant::now() < deadline,
                    "cursor not replaced before helper termination"
                );
                thread::sleep(Duration::from_millis(20));
            }
            session.child.kill().unwrap();
            drop(session);
            let deadline = Instant::now() + Duration::from_secs(5);
            while !fs::read_to_string(&result)
                .unwrap_or_default()
                .ends_with("bypassed")
            {
                assert!(
                    Instant::now() < deadline,
                    "detour did not safely bypass after helper termination"
                );
                assert!(
                    probe.try_wait().unwrap().is_none(),
                    "probe crashed after helper termination"
                );
                thread::sleep(Duration::from_millis(20));
            }
            probe.kill().unwrap();
            probe.wait().unwrap();
        }
        if let Some(value) = previous {
            std::env::set_var("APPDATA", value);
        } else {
            std::env::remove_var("APPDATA");
        }
        fs::remove_dir_all(temp).unwrap();
    }
}
