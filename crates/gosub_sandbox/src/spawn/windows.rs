//! Windows spawn backend: `CreateProcessAsUserW` with an extended startup info.

use std::ffi::c_void;
use std::io;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::Security::{SECURITY_CAPABILITIES, SID_AND_ATTRIBUTES};
use windows_sys::Win32::System::SystemServices::SE_GROUP_ENABLED;
use windows_sys::Win32::System::Threading::{
    CreateProcessAsUserW, CreateProcessW, DeleteProcThreadAttributeList, InitializeProcThreadAttributeList,
    TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject, EXTENDED_STARTUPINFO_PRESENT, INFINITE,
    PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
    STARTUPINFOEXW,
};

/// A spawned child process. Owns the process handle and closes it on drop.
pub struct Child {
    process: HANDLE,
    thread: HANDLE,
}

// SAFETY: a process handle is just a kernel object reference; it is not tied to
// the creating thread and may be waited on or closed from any thread.
unsafe impl Send for Child {}
unsafe impl Sync for Child {}

impl Child {
    /// Wait for the child to exit.
    pub fn wait(&mut self) -> io::Result<()> {
        // SAFETY: `process` is a valid handle owned by this struct.
        let rc = unsafe { WaitForSingleObject(self.process, INFINITE) };
        if rc != WAIT_OBJECT_0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Wait, and describe how the child ended - the unix backend distinguishes
    /// exit codes from fatal signals; here there are only exit codes.
    pub fn wait_describe(&mut self) -> String {
        match self.wait() {
            Ok(()) => "exited".to_string(),
            Err(e) => format!("could not be reaped: {e}"),
        }
    }

    /// The raw process handle, for the parent-side confinement hook.
    pub fn raw_handle(&self) -> HANDLE {
        self.process
    }

    /// Whether the child has exited, without blocking.
    pub fn try_wait(&mut self) -> io::Result<bool> {
        // SAFETY: `process` is a valid handle owned by this struct.
        match unsafe { WaitForSingleObject(self.process, 0) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(io::Error::last_os_error()),
        }
    }

    /// Best-effort terminate - used to abandon a child that has wedged (e.g. a
    /// decoder that stopped answering), so `wait` does not block forever.
    pub fn kill(&mut self) -> io::Result<()> {
        // SAFETY: `process` is a valid handle owned by this struct.
        if unsafe { TerminateProcess(self.process, 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        // SAFETY: both handles are valid and owned here; the child keeps
        // running if it has not exited - closing a handle does not kill it.
        unsafe {
            CloseHandle(self.thread);
            CloseHandle(self.process);
        }
    }
}

/// Quote one argument for the Windows command line.
fn quote(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => {
                backslashes += 1;
                out.push(c);
            }
            '"' => {
                // Double the run, then escape the quote itself.
                out.extend(std::iter::repeat_n('\\', backslashes + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                backslashes = 0;
                out.push(c);
            }
        }
    }
    // A trailing run precedes the closing quote, so it must be doubled too.
    out.extend(std::iter::repeat_n('\\', backslashes));
    out.push('"');
    out
}

/// A NUL-terminated UTF-16 buffer, as every `*W` API wants.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Spawn `exe` with `args`, handing `child_end` over as an inherited channel.
pub fn spawn(
    exe: &std::path::Path,
    args: &[&str],
    child_end: gosub_ipc::channel::Channel,
    isolation: crate::NamespaceIsolation,
    container: super::ContainerProfile<'_>,
) -> io::Result<Child> {
    // argv[0] is the program itself, by convention.
    let mut line = quote(&exe.to_string_lossy());
    for a in args {
        line.push(' ');
        line.push_str(&quote(a));
    }
    line.push(' ');
    line.push_str(&quote(&child_end.to_argv()));

    let mut exe_w = wide(&exe.to_string_lossy());
    let mut line_w = wide(&line);

    // The child's two pipe ends must be inheritable *and* on the list below.
    // The flag alone is process-wide; the list is what makes it per-child.
    let (rx, tx) = child_end.raw();
    gosub_ipc::channel::Channel::make_inheritable((rx, tx))?;
    let mut handles: [HANDLE; 2] = [rx as HANDLE, tx as HANDLE];

    // AppContainer (env-gated with GOSUB_WIN_APPCONTAINER while it is validated
    // on a real Windows host): put the child in a lowbox token instead of a
    // restricted primary token. The container is per role (the caller names
    // it), so a grant to one service never widens another's reach.
    let role = args.first().copied().unwrap_or("");
    let super::ContainerProfile {
        name: container,
        internet,
        fs_grant,
        // Memory is capped by the job object, one size for every role.
        data_limit: _,
        // Extra inherited descriptors are a Unix mechanism.
        extra_fds: _,
        // Task and file-size bounds are the job object's business, one size for every role.
        max_tasks: _,
        file_size_limit: _,
    } = container;
    let _ = isolation; // the container profile, not this mode, drives the split here
    let identity = if std::env::var_os("GOSUB_WIN_APPCONTAINER").is_some() {
        crate::app_container_identity(container, internet)
    } else {
        None
    };
    if let Some(id) = &identity {
        // A lowbox child can only load images the filesystem grants an app-package
        // SID; the PoC's executable does not by default, so grant it here (the
        // install-time ACL, done at spawn) or CreateProcess fails ERROR_FILE_NOT_FOUND.
        match crate::grant_app_package_execute(exe) {
            Ok(()) => eprintln!("[spawn] granted app-package read/exec on {}", exe.display()),
            Err(e) => eprintln!(
                "[spawn] could not grant app-package access to {} ({e}); \
                 AppContainer child may fail to load",
                exe.display()
            ),
        }
        // Give a filesystem service access to its own data path in its own
        // container, so it can still read/write it under the lowbox.
        if let Some((path, writable)) = &fs_grant {
            match crate::grant_container_path_access(path, id.container_sid(), *writable) {
                Ok(()) => eprintln!("[spawn] granted {role} container access to {}", path.display()),
                Err(e) => eprintln!(
                    "[spawn] could not grant {role} container access to {} ({e})",
                    path.display()
                ),
            }
        }
    }
    let attr_count: u32 = if identity.is_some() { 2 } else { 1 };

    // Attribute lists are sized by the API: ask with a null buffer, allocate,
    // then initialize for real.
    let mut size: usize = 0;
    // SAFETY: the documented two-call sizing protocol; failure here is
    // expected and reports the required size through `size`.
    unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), attr_count, 0, &mut size) };
    if size == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut attr_buf = vec![0u8; size];
    let attr_list = attr_buf.as_mut_ptr().cast();

    // SAFETY: `attr_list` points at `size` bytes, matching what the sizing call
    // asked for; `attr_count` attributes are declared.
    if unsafe { InitializeProcThreadAttributeList(attr_list, attr_count, 0, &mut size) } == 0 {
        return Err(io::Error::last_os_error());
    }

    // Attribute 1: the inherited-handle allowlist.
    // SAFETY: the handle array outlives the CreateProcess call below, which is
    // what the attribute list requires.
    let updated = unsafe {
        UpdateProcThreadAttribute(
            attr_list,
            0,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
            handles.as_mut_ptr().cast::<c_void>(),
            std::mem::size_of_val(&handles),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if updated == 0 {
        let e = io::Error::last_os_error();
        // SAFETY: initialized above.
        unsafe { DeleteProcThreadAttributeList(attr_list) };
        return Err(e);
    }

    // Attribute 2 (AppContainer): the SECURITY_CAPABILITIES. Built here so its
    // pointers - into `identity`'s SIDs and `cap_attrs` - stay valid across the
    // CreateProcess call below.
    let mut cap_attrs: Vec<SID_AND_ATTRIBUTES> = Vec::new();
    let mut sec_caps: SECURITY_CAPABILITIES = unsafe { std::mem::zeroed() };
    if let Some(id) = &identity {
        for &cap in id.capability_sids() {
            cap_attrs.push(SID_AND_ATTRIBUTES {
                Sid: cap,
                Attributes: SE_GROUP_ENABLED as u32,
            });
        }
        sec_caps.AppContainerSid = id.container_sid();
        sec_caps.CapabilityCount = cap_attrs.len() as u32;
        sec_caps.Capabilities = if cap_attrs.is_empty() {
            std::ptr::null_mut()
        } else {
            cap_attrs.as_mut_ptr()
        };
        // SAFETY: `sec_caps` and `cap_attrs` outlive CreateProcess below.
        let ok2 = unsafe {
            UpdateProcThreadAttribute(
                attr_list,
                0,
                PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
                std::ptr::addr_of!(sec_caps) as *mut c_void,
                std::mem::size_of::<SECURITY_CAPABILITIES>(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if ok2 == 0 {
            let e = io::Error::last_os_error();
            // SAFETY: initialized above.
            unsafe { DeleteProcThreadAttributeList(attr_list) };
            return Err(e);
        }
        eprintln!(
            "[spawn] AppContainer active (network={})",
            matches!(
                isolation,
                crate::NamespaceIsolation::None | crate::NamespaceIsolation::KeepNetwork
            )
        );
    }

    // SAFETY: zeroed is the documented "no overrides" state for STARTUPINFO.
    let mut si: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    si.lpAttributeList = attr_list;

    // SAFETY: zeroed out-param, filled by CreateProcess on success.
    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };

    // In an AppContainer the lowbox token *is* the confinement, so no restricted
    // primary token is used. Otherwise fall back to the token shapes, strongest
    // first (a host that refuses token creation should get a less confined
    // child, not no browser):
    let token = if identity.is_some() {
        None
    } else {
        crate::restricted_token()
    };

    // SAFETY: `exe_w` and `line_w` are NUL-terminated and live across the call;
    // `bInheritHandles = TRUE` is required for the handle list to apply.
    let ok = unsafe {
        match token {
            Some(t) => CreateProcessAsUserW(
                t,
                exe_w.as_mut_ptr(),
                line_w.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1, // inherit handles — scoped by the list above
                EXTENDED_STARTUPINFO_PRESENT,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::addr_of!(si).cast(),
                &mut pi,
            ),
            None => CreateProcessW(
                exe_w.as_mut_ptr(),
                line_w.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
                EXTENDED_STARTUPINFO_PRESENT,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::addr_of!(si).cast(),
                &mut pi,
            ),
        }
    };
    let err = io::Error::last_os_error();

    // SAFETY: initialized above and no longer needed either way.
    unsafe { DeleteProcThreadAttributeList(attr_list) };

    if let Some(t) = token {
        // SAFETY: built by `restricted_token`; the process (on success) holds
        // its own reference now, and on failure we simply free ours.
        unsafe { CloseHandle(t) };
    }
    // `identity` (and its SIDs) drops at the end of scope - after CreateProcess
    // has consumed the SECURITY_CAPABILITIES that pointed into it.

    if ok == 0 {
        return Err(err);
    }

    // The child holds its own copies now; drop ours so a dead child is seen as
    // EOF rather than a link the engine is itself holding open.
    drop(child_end);

    Ok(Child {
        process: pi.hProcess,
        thread: pi.hThread,
    })
}
