//! The owner socket: `detached-cli` reads and asks through it, from a Detached terminal tab only.
//! One JSON line each way: an `Ask` in, a `Result<Value, String>` out.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::env::Env;
use crate::error::{Error, Result};
use crate::os;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Ask {
    /// A read with the owner's view.
    Q(String),
    /// An env profile's values, for `env run`.
    Env(String),
}

/// Answers a tab's ask; the tab is its id from `Tabs::add`.
pub type Handler = Box<dyn Fn(u64, Ask) -> Result<Value> + Send + Sync>;

/// A terminal tab's variable that tells `detached-cli` where the socket is.
pub const SOCKET_VAR: &str = "DETACHED_SOCKET";

pub const REFUSED: &str = "Refused: detached-cli answers only in a Detached terminal tab.";

/// The live terminal tabs: only their shells' sessions (Unix) or jobs (Windows) may connect.
#[derive(Default)]
pub struct Tabs(Mutex<Vec<(u64, Weak<os::Session>)>>);

impl Tabs {
    pub fn add(&self, id: u64, shell: &Arc<os::Session>) {
        self.0.lock().unwrap().push((id, Arc::downgrade(shell)));
    }

    fn of(&self, pid: u32) -> Option<u64> {
        let mut tabs = self.0.lock().unwrap();
        tabs.retain(|t| t.1.strong_count() > 0);
        let holds = |t: &&(u64, Weak<os::Session>)| t.1.upgrade().is_some_and(|s| s.holds(pid));
        tabs.iter().find(holds).map(|t| t.0)
    }
}

/// `data/run/detached.sock`; on Windows a pipe named after the data dir.
pub fn socket(data: &Path) -> PathBuf {
    #[cfg(unix)]
    return data.join("run").join("detached.sock");
    #[cfg(windows)]
    {
        use sha2::Digest;
        let hash = sha2::Sha256::digest(data.as_os_str().as_encoded_bytes());
        let hex: String = hash[..8].iter().map(|b| format!("{b:02x}")).collect();
        PathBuf::from(format!(r"\\.\pipe\detached-{hex}"))
    }
}

/// Listens until dropped.
pub struct Server {
    path: PathBuf,
    stop: Arc<AtomicBool>,
}

pub fn serve(path: &Path, tabs: Arc<Tabs>, handle: Handler) -> Result<Server> {
    let mut listener = sys::listen(path)?;
    let stop = Arc::new(AtomicBool::new(false));
    let (handle, stopped) = (Arc::new(handle), stop.clone());
    std::thread::Builder::new()
        .name("owner socket".into())
        .spawn(move || {
            loop {
                let accepted = listener.accept();
                if stopped.load(Ordering::SeqCst) {
                    return;
                }
                match accepted {
                    Ok((conn, pid)) => {
                        let tab = pid.and_then(|p| tabs.of(p));
                        let handle = handle.clone();
                        let _ =
                            std::thread::Builder::new().spawn(move || answer(conn, tab, &handle));
                    }
                    Err(e) => {
                        eprintln!("owner socket: {e}");
                        // Out of descriptors lasts a while: no busy loop meanwhile.
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                }
            }
        })?;
    Ok(Server {
        path: path.into(),
        stop,
    })
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = sys::connect(&self.path);
        #[cfg(unix)]
        let _ = std::fs::remove_file(&self.path);
    }
}

fn answer(conn: sys::Conn, tab: Option<u64>, handle: &Handler) {
    let mut reader = BufReader::new((&conn).take(1 << 20));
    let mut line = String::new();
    let ask = reader.read_line(&mut line).map_err(Error::from);
    let reply = match (
        tab,
        ask.and_then(|_| Ok(serde_json::from_str::<Ask>(&line)?)),
    ) {
        (None, _) => Err(REFUSED.to_string()),
        (Some(tab), Ok(ask)) => handle(tab, ask).map_err(|e| e.to_string()),
        (_, Err(e)) => Err(e.to_string()),
    };
    let reply = serde_json::to_string(&reply).unwrap_or_default() + "\n";
    if (&conn).write_all(reply.as_bytes()).is_ok() {
        // Until the client has read it: a pipe closed sooner drops what it holds.
        let _ = io::copy(&mut reader, &mut io::sink());
    }
}

/// Sends `ask` to the app listening at `path`.
pub fn ask(path: &Path, ask: &Ask) -> Result<Value> {
    let conn =
        sys::connect(path).map_err(|e| Error::Other(format!("Detached is not running ({e}).")))?;
    (&conn).write_all((serde_json::to_string(ask)? + "\n").as_bytes())?;
    let mut line = String::new();
    BufReader::new(&conn).read_line(&mut line)?;
    serde_json::from_str::<std::result::Result<Value, String>>(&line)?.map_err(Error::Other)
}

/// Puts `detached-cli` from `exe_dir` on the PATH: a link in `~/.local/bin` on Unix, `exe_dir` in
/// the user's Path on Windows. Says where, and whether the login PATH has it yet.
pub fn install(exe_dir: &Path, env: &Env) -> Result<String> {
    let cli = exe_dir.join(format!("detached-cli{}", std::env::consts::EXE_SUFFIX));
    if !cli.is_file() {
        return Err(Error::Other(format!(
            "No {} beside Detached.",
            cli.display()
        )));
    }
    let home = env.vars.iter().find(|(k, _)| k == "HOME");
    let home = home.map(|(_, v)| PathBuf::from(v)).or_else(dirs::home_dir);
    let dir = match (cfg!(windows), home) {
        (true, _) => exe_dir.to_path_buf(),
        (false, Some(home)) => home.join(".local").join("bin"),
        (false, None) => return Err(Error::Other("No home folder.".into())),
    };
    #[cfg(unix)]
    {
        std::fs::create_dir_all(&dir)?;
        let link = dir.join("detached-cli");
        if std::fs::read_link(&link).ok().as_ref() != Some(&cli) {
            if link.exists() {
                let e = format!("{} is there already; move it away first.", link.display());
                return Err(Error::Other(e));
            }
            if link.symlink_metadata().is_ok() {
                std::fs::remove_file(&link)?;
            }
            std::os::unix::fs::symlink(&cli, &link)?;
        }
    }
    #[cfg(windows)]
    sys::add_to_user_path(&dir.display().to_string())?;
    let todo = if cfg!(windows) {
        "Terminals see it once Detached restarts."
    } else {
        "Add that folder to the PATH."
    };
    Ok(match std::env::split_paths(&env.path).any(|p| p == dir) {
        true => format!("detached-cli is on the PATH, in {}.", dir.display()),
        false => format!("detached-cli is in {}. {todo}", dir.display()),
    })
}

/// The user's raw Path with `dir` added, and whether it expands `%VAR%`; None when it has `dir`.
#[cfg(any(windows, test))]
fn path_with(old: Option<(&str, bool)>, dir: &str) -> Option<(String, bool)> {
    let (path, expand) = old.unwrap_or(("", true));
    let trim = |e: &str| e.trim_end_matches('\\').to_lowercase();
    if path.split(';').any(|e| trim(e) == trim(dir)) {
        return None;
    }
    let path = path.trim_end_matches(';');
    let sep = if path.is_empty() { "" } else { ";" };
    Some((format!("{path}{sep}{dir}"), expand))
}

#[cfg(unix)]
mod sys {
    use std::io;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::Path;

    pub type Conn = UnixStream;

    pub struct Listener(UnixListener);

    pub fn listen(path: &Path) -> io::Result<Listener> {
        use std::fs::{DirBuilder, Permissions, set_permissions};
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let dir = path.parent().unwrap_or(Path::new("."));
        DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        set_permissions(dir, Permissions::from_mode(0o700))?;
        if connect(path).is_ok() {
            let e = "another Detached answers on this data folder";
            return Err(io::Error::new(io::ErrorKind::AddrInUse, e));
        }
        let _ = std::fs::remove_file(path);
        let listener = UnixListener::bind(path)?;
        set_permissions(path, Permissions::from_mode(0o600))?;
        Ok(Listener(listener))
    }

    pub fn connect(path: &Path) -> io::Result<Conn> {
        UnixStream::connect(path)
    }

    impl Listener {
        pub fn accept(&mut self) -> io::Result<(Conn, Option<u32>)> {
            let (conn, _) = self.0.accept()?;
            let pid = peer(&conn);
            Ok((conn, pid))
        }
    }

    fn peer(conn: &UnixStream) -> Option<u32> {
        use std::os::fd::AsRawFd;
        #[cfg(target_os = "macos")]
        let (level, name, mut v) = (libc::SOL_LOCAL, libc::LOCAL_PEERPID, 0 as libc::pid_t);
        #[cfg(not(target_os = "macos"))]
        let (level, name, mut v) = (
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            libc::ucred {
                pid: 0,
                uid: 0,
                gid: 0,
            },
        );
        let mut len = size_of_val(&v) as libc::socklen_t;
        let fd = conn.as_raw_fd();
        let r = unsafe { libc::getsockopt(fd, level, name, (&raw mut v).cast(), &mut len) };
        #[cfg(not(target_os = "macos"))]
        let v = v.pid;
        (r == 0 && v > 0).then_some(v as u32)
    }
}

#[cfg(windows)]
mod sys {
    use std::fs::File;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::FromRawHandle;
    use std::path::Path;
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_NO_DATA, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED,
        GetLastError, HANDLE, INVALID_HANDLE_VALUE, LocalFree,
    };
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{
        GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
        TokenUser,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX,
    };
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, PIPE_READMODE_BYTE,
        PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
        WaitNamedPipeW,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    pub type Conn = File;

    pub struct Listener {
        name: Vec<u16>,
        sd: PSECURITY_DESCRIPTOR,
        pipe: HANDLE,
    }

    // SAFETY: the handle and the descriptor are used from the accept thread alone.
    unsafe impl Send for Listener {}

    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        s.encode_wide().chain([0]).collect()
    }

    fn check(ok: i32) -> io::Result<()> {
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn listen(path: &Path) -> io::Result<Listener> {
        let name = wide(path.as_os_str());
        let sd = owner_only()?;
        let pipe = create(&name, sd, true)?;
        Ok(Listener { name, sd, pipe })
    }

    pub fn connect(path: &Path) -> io::Result<Conn> {
        let open = || File::options().read(true).write(true).open(path);
        for _ in 0..3 {
            match open() {
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => unsafe {
                    WaitNamedPipeW(wide(path.as_os_str()).as_ptr(), 2000);
                },
                r => return r,
            }
        }
        open()
    }

    impl Listener {
        pub fn accept(&mut self) -> io::Result<(Conn, Option<u32>)> {
            let ok = unsafe { ConnectNamedPipe(self.pipe, null_mut()) };
            let err = unsafe { GetLastError() };
            if ok == 0 && err != ERROR_PIPE_CONNECTED && err != ERROR_NO_DATA {
                return Err(io::Error::from_raw_os_error(err as i32));
            }
            let next = create(&self.name, self.sd, false)?;
            let conn = std::mem::replace(&mut self.pipe, next);
            let mut pid = 0;
            let known = unsafe { GetNamedPipeClientProcessId(conn, &mut pid) } != 0;
            Ok((unsafe { File::from_raw_handle(conn) }, known.then_some(pid)))
        }
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.pipe);
                LocalFree(self.sd);
            }
        }
    }

    fn create(name: &[u16], sd: PSECURITY_DESCRIPTOR, first: bool) -> io::Result<HANDLE> {
        let sa = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd,
            bInheritHandle: 0,
        };
        let first = if first {
            FILE_FLAG_FIRST_PIPE_INSTANCE
        } else {
            0
        };
        let mode = PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS;
        let pipe = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_DUPLEX | first,
                mode,
                PIPE_UNLIMITED_INSTANCES,
                4096,
                4096,
                0,
                &sa,
            )
        };
        if pipe == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        Ok(pipe)
    }

    /// Adds `dir` to the user's Path as it is stored, unexpanded, and tells running programs.
    pub fn add_to_user_path(dir: &str) -> io::Result<()> {
        use windows_sys::Win32::System::Registry::{
            HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_EXPAND_SZ, REG_SZ, RegCloseKey,
            RegCreateKeyExW, RegQueryValueExW, RegSetValueExW,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
        };
        let win = |r: u32| match r {
            0 => Ok(()),
            r => Err(io::Error::from_raw_os_error(r as i32)),
        };
        let (env, name) = (wide("Environment".as_ref()), wide("Path".as_ref()));
        let mut key = null_mut();
        win(unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                env.as_ptr(),
                0,
                null(),
                0,
                KEY_QUERY_VALUE | KEY_SET_VALUE,
                null(),
                &mut key,
                null_mut(),
            )
        })?;
        let added = (|| -> io::Result<bool> {
            let (mut kind, mut len) = (0, 0);
            let query = |kind: &mut u32, data: *mut u8, len: &mut u32| unsafe {
                RegQueryValueExW(key, name.as_ptr(), null(), kind, data, len)
            };
            let old = match query(&mut kind, null_mut(), &mut len) {
                ERROR_FILE_NOT_FOUND => None,
                r => {
                    win(r)?;
                    let mut buf = vec![0u16; len as usize / 2 + 1];
                    win(query(&mut kind, buf.as_mut_ptr().cast(), &mut len))?;
                    buf.truncate(len as usize / 2);
                    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                    Some(String::from_utf16_lossy(&buf[..n]))
                }
            };
            let old = old.as_deref().map(|p| (p, kind == REG_EXPAND_SZ));
            let Some((path, expand)) = super::path_with(old, dir) else {
                return Ok(false);
            };
            let data = wide(path.as_ref());
            let kind = if expand { REG_EXPAND_SZ } else { REG_SZ };
            let len = (data.len() * 2) as u32;
            win(unsafe { RegSetValueExW(key, name.as_ptr(), 0, kind, data.as_ptr().cast(), len) })?;
            Ok(true)
        })();
        unsafe { RegCloseKey(key) };
        if added? {
            let mut r = 0;
            let env = env.as_ptr() as isize;
            unsafe {
                SendMessageTimeoutW(
                    HWND_BROADCAST,
                    WM_SETTINGCHANGE,
                    0,
                    env,
                    SMTO_ABORTIFHUNG,
                    5000,
                    &mut r,
                )
            };
        }
        Ok(())
    }

    /// A DACL that grants the current user alone, and no one inherits.
    fn owner_only() -> io::Result<PSECURITY_DESCRIPTOR> {
        unsafe {
            let mut token = null_mut();
            check(OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_QUERY,
                &mut token,
            ))?;
            // Aligned for TOKEN_USER's pointer.
            let mut buf = [0usize; 64];
            let (size, mut len) = (size_of_val(&buf) as u32, 0);
            let got =
                GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), size, &mut len);
            CloseHandle(token);
            check(got)?;
            let user = &*buf.as_ptr().cast::<TOKEN_USER>();
            let mut sid = null_mut();
            check(ConvertSidToStringSidW(user.User.Sid, &mut sid))?;
            let n = (0..).take_while(|&i| *sid.add(i) != 0).count();
            let sid_text = String::from_utf16_lossy(std::slice::from_raw_parts(sid, n));
            LocalFree(sid.cast());
            let sddl = wide(format!("D:P(A;;GA;;;{sid_text})").as_ref());
            let mut sd = null_mut();
            check(ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut sd,
                null_mut(),
            ))?;
            Ok(sd)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_a_process_in_a_tab_s_session_and_refuses_one_outside() {
        let dir = tempfile::tempdir().unwrap();
        let path = socket(dir.path());
        let tabs = Arc::new(Tabs::default());
        let handle: Handler =
            Box::new(|tab, ask| Ok(serde_json::json!({ "tab": tab, "ask": ask })));
        let _server = serve(&path, tabs.clone(), handle).unwrap();
        let q = Ask::Q("{ schema }".into());
        let refused = ask(&path, &q).unwrap_err().to_string();
        assert_eq!(refused, REFUSED);
        // This process's own session, or on Windows a job it joins.
        #[cfg(unix)]
        let me = unsafe { libc::getsid(0) } as u32;
        #[cfg(windows)]
        let me = std::process::id();
        let shell = Arc::new(os::Session::new(me));
        tabs.add(7, &shell);
        let answer = ask(&path, &q).unwrap();
        assert_eq!(
            answer,
            serde_json::json!({ "tab": 7, "ask": { "q": "{ schema }" } })
        );
        drop(shell);
        assert_eq!(ask(&path, &q).unwrap_err().to_string(), REFUSED);
    }

    #[cfg(unix)]
    #[test]
    fn install_links_the_cli_into_local_bin_and_keeps_a_file_that_is_not_its_link() {
        let dir = tempfile::tempdir().unwrap();
        let (exe, home) = (dir.path().join("app"), dir.path().join("home"));
        let env = |path: String| Env {
            path,
            git: "git".into(),
            vars: vec![("HOME".into(), home.display().to_string())],
        };
        assert!(install(&exe, &env("/usr/bin".into())).is_err());
        std::fs::create_dir(&exe).unwrap();
        std::fs::write(exe.join("detached-cli"), "").unwrap();
        let said = install(&exe, &env("/usr/bin".into())).unwrap();
        assert!(said.contains("Add that folder"), "{said}");
        let link = home.join(".local/bin/detached-cli");
        assert_eq!(std::fs::read_link(&link).unwrap(), exe.join("detached-cli"));
        let path = format!("/usr/bin:{}", home.join(".local/bin").display());
        assert!(
            install(&exe, &env(path.clone()))
                .unwrap()
                .contains("is on the PATH")
        );
        let theirs = dir.path().join("theirs");
        std::fs::write(&theirs, "").unwrap();
        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(&theirs, &link).unwrap();
        let refused = install(&exe, &env(path.clone())).unwrap_err().to_string();
        assert!(refused.contains(&link.display().to_string()), "{refused}");
        assert_eq!(std::fs::read_link(&link).unwrap(), theirs);
        std::fs::remove_file(&theirs).unwrap();
        install(&exe, &env(path.clone())).unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), exe.join("detached-cli"));
        std::fs::remove_file(&link).unwrap();
        std::fs::write(&link, "a script").unwrap();
        assert!(install(&exe, &env(path)).is_err());
        assert_eq!(std::fs::read_to_string(&link).unwrap(), "a script");
    }

    #[test]
    fn a_second_server_is_refused_and_the_first_removes_its_socket() {
        let dir = tempfile::tempdir().unwrap();
        let path = socket(dir.path());
        let null = || -> Handler { Box::new(|_, _| Ok(Value::Null)) };
        let first = serve(&path, Default::default(), null()).unwrap();
        assert!(serve(&path, Default::default(), null()).is_err());
        let q = Ask::Q("x".into());
        assert_eq!(ask(&path, &q).unwrap_err().to_string(), REFUSED);
        drop(first);
        #[cfg(unix)]
        assert!(!path.exists());
    }

    #[test]
    fn a_path_keeps_its_entries_unexpanded_and_gets_the_folder_once() {
        let apps = r"%USERPROFILE%\AppData\Local\Microsoft\WindowsApps";
        let dir = r"C:\Program Files\Detached";
        let added = path_with(Some((&format!("{apps};"), true)), dir);
        assert_eq!(added, Some((format!("{apps};{dir}"), true)));
        let (path, _) = added.unwrap();
        assert_eq!(path_with(Some((&path, true)), dir), None);
        let other_case = format!("{apps};{}\\", dir.to_uppercase());
        assert_eq!(path_with(Some((&other_case, true)), dir), None);
        assert_eq!(
            path_with(Some(("C:\\bin", false)), dir),
            Some((format!("C:\\bin;{dir}"), false))
        );
        assert_eq!(path_with(None, dir), Some((dir.into(), true)));
        assert_eq!(path_with(Some(("", false)), dir), Some((dir.into(), false)));
    }

    #[cfg(unix)]
    #[test]
    fn the_socket_is_the_owner_s_alone() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = socket(dir.path());
        let _server = serve(&path, Default::default(), Box::new(|_, _| Ok(Value::Null))).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(path.parent().unwrap()), 0o700);
        assert_eq!(mode(&path), 0o600);
    }
}
