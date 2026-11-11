use std::{ffi::OsStr, io, os::windows::ffi::OsStrExt, ptr};
use windows_sys::Win32::{
    Foundation::*,
    System::{JobObjects::*, Threading::*},
    UI::WindowsAndMessaging::*,
};

pub fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}
pub fn error(message: &str) {
    unsafe {
        MessageBoxW(
            ptr::null_mut(),
            wide(message).as_ptr(),
            wide("AirBridge").as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

pub struct Instance(HANDLE);
impl Instance {
    pub fn acquire() -> io::Result<Option<Self>> {
        let handle = unsafe {
            CreateMutexW(
                ptr::null(),
                0,
                wide("Local\\AirBridge.Receiver.Launcher").as_ptr(),
            )
        };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe {
                CloseHandle(handle);
            }
            return Ok(None);
        }
        // Existence, rather than mutex ownership, is the single-instance guard. A crashed
        // owner's abandoned mutex must not let two living supervisors claim ownership.
        Ok(Some(Self(handle)))
    }
}
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub struct Job(HANDLE);
// The handle is only owned by the supervisor thread; Windows kernel handles are transferable.
unsafe impl Send for Job {}
impl Job {
    pub fn new() -> io::Result<Self> {
        let h = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if h.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = Self(h);
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if unsafe {
            SetInformationJobObject(
                h,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of_val(&info) as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }
    pub fn spawn(
        &self,
        exe: &std::path::Path,
        root: &std::path::Path,
        port: u16,
    ) -> io::Result<Process> {
        unsafe {
            let mut size = 0;
            InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut size);
            let mut storage = vec![0usize; size.div_ceil(std::mem::size_of::<usize>())];
            let attributes = storage.as_mut_ptr().cast();
            if InitializeProcThreadAttributeList(attributes, 1, 0, &mut size) == 0 {
                return Err(io::Error::last_os_error());
            }
            let result = (|| {
                // Atomically attach the receiver to the job during process creation.
                // A supervisor killed between spawn and assignment cannot orphan it.
                if UpdateProcThreadAttribute(
                    attributes,
                    0,
                    PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
                    &self.0 as *const _ as *const _,
                    std::mem::size_of::<HANDLE>(),
                    ptr::null_mut(),
                    ptr::null(),
                ) == 0
                {
                    return Err(io::Error::last_os_error());
                }
                let mut startup: STARTUPINFOEXW = std::mem::zeroed();
                startup.StartupInfo.cb = std::mem::size_of_val(&startup) as u32;
                startup.lpAttributeList = attributes;
                let mut info: PROCESS_INFORMATION = std::mem::zeroed();
                let app = wide(exe.as_os_str());
                let mut command = wide(format!("\"{}\" --receiver --port {port}", exe.display()));
                let directory = wide(root.as_os_str());
                if CreateProcessW(
                    app.as_ptr(),
                    command.as_mut_ptr(),
                    ptr::null(),
                    ptr::null(),
                    0,
                    CREATE_NO_WINDOW | EXTENDED_STARTUPINFO_PRESENT,
                    ptr::null(),
                    directory.as_ptr(),
                    &startup.StartupInfo,
                    &mut info,
                ) == 0
                {
                    return Err(io::Error::last_os_error());
                }
                CloseHandle(info.hThread);
                Ok(Process {
                    handle: info.hProcess,
                    pid: info.dwProcessId,
                })
            })();
            DeleteProcThreadAttributeList(attributes);
            result
        }
    }
    pub fn terminate(&self) {
        unsafe {
            TerminateJobObject(self.0, 1);
        }
    }
}
pub struct Process {
    handle: HANDLE,
    pid: u32,
}
impl Process {
    pub fn id(&self) -> u32 {
        self.pid
    }
    pub fn try_wait(&self) -> io::Result<Option<u32>> {
        match unsafe { WaitForSingleObject(self.handle, 0) } {
            WAIT_TIMEOUT => Ok(None),
            WAIT_OBJECT_0 => {
                let mut code = 0;
                if unsafe { GetExitCodeProcess(self.handle, &mut code) } == 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(Some(code))
            }
            _ => Err(io::Error::last_os_error()),
        }
    }
    pub fn wait(&self) -> io::Result<()> {
        if unsafe { WaitForSingleObject(self.handle, 5000) } == WAIT_OBJECT_0 {
            Ok(())
        } else {
            Err(io::Error::other("Process termination timed out"))
        }
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.handle);
        }
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub fn focus() {
    unsafe {
        let h = FindWindowW(wide("AirBridge.Rust.Receiver").as_ptr(), ptr::null());
        if !h.is_null() {
            if IsIconic(h) != 0 {
                ShowWindow(h, SW_RESTORE);
            }
            SetForegroundWindow(h);
        }
    }
}
pub fn close_receiver(pid: u32) {
    unsafe extern "system" fn callback(h: HWND, l: LPARAM) -> i32 {
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(h, &mut pid);
        }
        if pid == l as u32 {
            unsafe {
                PostMessageW(h, WM_CLOSE, 0, 0);
            }
        }
        1
    }
    unsafe {
        EnumWindows(Some(callback), pid as isize);
    }
}
