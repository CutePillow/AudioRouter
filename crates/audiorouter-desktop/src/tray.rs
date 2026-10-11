use std::mem::zeroed;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::UI::Shell::{
    DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass, Shell_NotifyIconW, NIF_ICON,
    NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, LoadIconW, LoadImageW, PostMessageW,
    SetForegroundWindow, TrackPopupMenu, IDI_APPLICATION, IMAGE_ICON, LR_DEFAULTSIZE,
    LR_LOADFROMFILE, MF_STRING, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_APP, WM_LBUTTONDBLCLK,
    WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP,
};

const TRAY_ID: u32 = 1;
const TRAY_CALLBACK_MESSAGE: u32 = WM_APP + 41;
const TRAY_SUBCLASS_ID: usize = 0x4152;
const MENU_SHOW: usize = 1;
const MENU_EXIT: usize = 2;

#[derive(Default)]
struct TrayActions {
    restore_requested: AtomicBool,
    quit_requested: AtomicBool,
}

pub(crate) struct TrayController {
    hwnd: HWND,
    data: NOTIFYICONDATAW,
    actions: Arc<TrayActions>,
    visible: bool,
}

impl TrayController {
    pub(crate) fn new(hwnd: HWND) -> Option<Self> {
        let actions = Arc::new(TrayActions::default());
        let subclass_data = Arc::as_ptr(&actions) as usize;
        let subclassed = unsafe {
            SetWindowSubclass(
                hwnd,
                Some(tray_window_proc),
                TRAY_SUBCLASS_ID,
                subclass_data,
            )
        };
        if subclassed == 0 {
            return None;
        }

        let mut data: NOTIFYICONDATAW = unsafe { zeroed() };
        data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = hwnd;
        data.uID = TRAY_ID;
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        data.uCallbackMessage = TRAY_CALLBACK_MESSAGE;
        data.hIcon = load_tray_icon();
        for (slot, value) in data.szTip.iter_mut().zip("AudioRouter".encode_utf16()) {
            *slot = value;
        }

        Some(Self {
            hwnd,
            data,
            actions,
            visible: false,
        })
    }

    pub(crate) fn set_visible(&mut self, visible: bool) -> bool {
        if self.visible == visible {
            return true;
        }
        let message = if visible { NIM_ADD } else { NIM_DELETE };
        let success = unsafe { Shell_NotifyIconW(message, &self.data) } != 0;
        if success {
            self.visible = visible;
        }
        success
    }

    pub(crate) fn take_restore_request(&self) -> bool {
        self.actions.restore_requested.swap(false, Ordering::AcqRel)
    }

    pub(crate) fn take_quit_request(&self) -> bool {
        self.actions.quit_requested.swap(false, Ordering::AcqRel)
    }
}

fn load_tray_icon() -> windows_sys::Win32::UI::WindowsAndMessaging::HICON {
    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            let icon_path = directory.join("audiorouter.ico");
            if let Some(icon) = load_icon_from_file(&icon_path) {
                return icon;
            }
        }
    }
    unsafe { LoadIconW(null_mut(), IDI_APPLICATION) }
}

fn load_icon_from_file(path: &Path) -> Option<windows_sys::Win32::UI::WindowsAndMessaging::HICON> {
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let icon = unsafe {
        LoadImageW(
            null_mut(),
            path.as_ptr(),
            IMAGE_ICON,
            0,
            0,
            LR_LOADFROMFILE | LR_DEFAULTSIZE,
        )
    };
    (!icon.is_null()).then_some(icon)
}

impl Drop for TrayController {
    fn drop(&mut self) {
        self.set_visible(false);
        unsafe {
            RemoveWindowSubclass(self.hwnd, Some(tray_window_proc), TRAY_SUBCLASS_ID);
        }
    }
}

unsafe extern "system" fn tray_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    reference_data: usize,
) -> LRESULT {
    if message == TRAY_CALLBACK_MESSAGE && wparam as u32 == TRAY_ID {
        let actions = &*(reference_data as *const TrayActions);
        match lparam as u32 {
            WM_LBUTTONUP | WM_LBUTTONDBLCLK => {
                actions.restore_requested.store(true, Ordering::Release);
            }
            WM_RBUTTONUP => show_context_menu(hwnd, actions),
            _ => {}
        }
        return 0;
    }
    DefSubclassProc(hwnd, message, wparam, lparam)
}

unsafe fn show_context_menu(hwnd: HWND, actions: &TrayActions) {
    let menu = CreatePopupMenu();
    if menu.is_null() {
        return;
    }
    let show_label = "打开 AudioRouter\0".encode_utf16().collect::<Vec<_>>();
    let exit_label = "退出 AudioRouter\0".encode_utf16().collect::<Vec<_>>();
    AppendMenuW(menu, MF_STRING, MENU_SHOW, show_label.as_ptr());
    AppendMenuW(menu, MF_STRING, MENU_EXIT, exit_label.as_ptr());

    let mut point = POINT { x: 0, y: 0 };
    GetCursorPos(&mut point);
    SetForegroundWindow(hwnd);
    let command = TrackPopupMenu(
        menu,
        TPM_RETURNCMD | TPM_RIGHTBUTTON,
        point.x,
        point.y,
        0,
        hwnd,
        null(),
    );
    DestroyMenu(menu);
    PostMessageW(hwnd, WM_NULL, 0, 0);

    match command as usize {
        MENU_SHOW => actions.restore_requested.store(true, Ordering::Release),
        MENU_EXIT => actions.quit_requested.store(true, Ordering::Release),
        _ => {}
    }
}
