#![cfg_attr(not(test), windows_subsystem = "windows")]

use std::cell::RefCell;
use std::collections::HashMap;
use std::env;
use std::mem::{size_of, zeroed};
use std::ptr::null_mut;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, S_OK, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, HMONITOR, MONITORINFO,
    MONITOR_DEFAULTTONEAREST,
};
use windows_sys::Win32::System::Console::{
    AllocConsole, FreeConsole, GetConsoleMode, GetStdHandle, SetConsoleMode,
    ENABLE_EXTENDED_FLAGS, ENABLE_QUICK_EDIT_MODE, STD_INPUT_HANDLE,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows_sys::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    keybd_event, RegisterHotKey, UnregisterHotKey, KEYEVENTF_KEYUP, MOD_ALT, MOD_NOREPEAT,
    VK_OEM_4, VK_OEM_6,
};
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    DispatchMessageW, GetCursorPos, GetForegroundWindow, GetMessageW, GetShellWindow,
    GetWindowLongW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, IsZoomed, KillTimer, LoadIconW,
    PostQuitMessage, RegisterClassW, SendMessageTimeoutW, SetForegroundWindow, SetTimer,
    SetWindowPos, ShowWindow, ShowWindowAsync, TrackPopupMenu, TranslateMessage,
    EVENT_OBJECT_DESTROY, EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND,
    EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MOVESIZEEND, EVENT_SYSTEM_MOVESIZESTART, GWL_EXSTYLE,
    GWL_STYLE, HWND_NOTOPMOST, HWND_TOPMOST, IDI_APPLICATION, MF_SEPARATOR, MF_STRING, MSG,
    OBJID_WINDOW, SC_RESTORE, SMTO_ABORTIFHUNG, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_NOZORDER, SW_MINIMIZE, SW_RESTORE, SW_SHOWNOACTIVATE, TPM_BOTTOMALIGN, TPM_LEFTALIGN,
    TPM_RIGHTBUTTON, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_COMMAND, WM_DESTROY,
    WM_HOTKEY, WM_LBUTTONDBLCLK, WM_RBUTTONUP, WM_SYSCOMMAND, WM_TIMER, WM_USER, WNDCLASSW,
    WS_EX_TOOLWINDOW, WS_OVERLAPPEDWINDOW, WS_THICKFRAME,
};

#[link(name = "kernel32")]
extern "system" {
    fn GetUserDefaultUILanguage() -> u16;
    fn GetCurrentThreadId() -> u32;
}

#[link(name = "user32")]
extern "system" {
    fn AttachThreadInput(id_attach: u32, id_attach_to: u32, f_attach: i32) -> i32;
}

// Hotkey IDs / 快捷键 ID
const HK_TOGGLE_WINDOW: i32 = 1; // Alt + G: Add / Remove current window & auto-tile
const HK_MIN_RESTORE: i32 = 2;   // Alt + H: Hide (Minimize) / Restore entire group
const HK_CYCLE_REGION: i32 = 3;  // Alt + S: Cycle tiling region
const HK_RETILE: i32 = 4;        // Alt + T: Manually re-tile current group
const HK_CLEAR_GROUP: i32 = 5;   // Alt + C: Clear window group
const HK_QUIT: i32 = 6;          // Alt + Q: Quit program
const HK_TOGGLE_LANG: i32 = 7;   // Alt + L: Switch UI language (EN / ZH)
const HK_MOVE_PREV: i32 = 8;     // Alt + [: Move active window to previous slot
const HK_MOVE_NEXT: i32 = 9;     // Alt + ]: Move active window to next slot

// System Tray & Timer constants
const WM_TRAYICON: u32 = WM_USER + 1;
const TRAY_ICON_ID: u32 = 1;
const TIMER_STATE_POLL: usize = 100;
const TIMER_POST_SWAP: usize = 101;

// Tray Menu Command IDs
const CMD_RETILE: usize = 1001;
const CMD_MIN_RESTORE: usize = 1002;
const CMD_CYCLE_REGION: usize = 1003;
const CMD_CLEAR_GROUP: usize = 1004;
const CMD_TOGGLE_CONSOLE: usize = 1005;
const CMD_TOGGLE_LANG: usize = 1006;
const CMD_QUIT: usize = 1007;

/// UI Language / 界面语言
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lang {
    En,
    Zh,
}

impl Lang {
    fn detect() -> Self {
        for arg in env::args().skip(1) {
            match arg.to_ascii_lowercase().as_str() {
                "--en" | "-e" | "en" | "english" => return Lang::En,
                "--zh" | "-z" | "zh" | "cn" | "chinese" => return Lang::Zh,
                _ => {}
            }
        }
        if let Ok(exe_path) = env::current_exe() {
            if let Some(stem) = exe_path.file_stem().and_then(|s| s.to_str()) {
                let lower = stem.to_ascii_lowercase();
                if lower.ends_with("-en") || lower.ends_with("_en") {
                    return Lang::En;
                }
                if lower.ends_with("-zh") || lower.ends_with("_zh") || lower.ends_with("-cn") {
                    return Lang::Zh;
                }
            }
        }
        let lang_id = unsafe { GetUserDefaultUILanguage() };
        if (lang_id & 0x3ff) == 0x04 {
            Lang::Zh
        } else {
            Lang::En
        }
    }

    fn toggle(self) -> Self {
        match self {
            Lang::En => Lang::Zh,
            Lang::Zh => Lang::En,
        }
    }
}

/// Target tiling region mode / 平铺的目标区域模式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TileRegion {
    FullWorkArea,
    LeftHalf,
    RightHalf,
    TopHalf,
    BottomHalf,
}

impl TileRegion {
    fn next(self) -> Self {
        match self {
            TileRegion::FullWorkArea => TileRegion::LeftHalf,
            TileRegion::LeftHalf => TileRegion::RightHalf,
            TileRegion::RightHalf => TileRegion::TopHalf,
            TileRegion::TopHalf => TileRegion::BottomHalf,
            TileRegion::BottomHalf => TileRegion::FullWorkArea,
        }
    }

    fn name(self, lang: Lang) -> &'static str {
        match (self, lang) {
            (TileRegion::FullWorkArea, Lang::Zh) => "全屏工作区 (Full WorkArea)",
            (TileRegion::LeftHalf, Lang::Zh) => "左半屏 (Left Half)",
            (TileRegion::RightHalf, Lang::Zh) => "右半屏 (Right Half)",
            (TileRegion::TopHalf, Lang::Zh) => "上半屏 (Top Half)",
            (TileRegion::BottomHalf, Lang::Zh) => "下半屏 (Bottom Half)",
            (TileRegion::FullWorkArea, Lang::En) => "Full WorkArea",
            (TileRegion::LeftHalf, Lang::En) => "Left Half",
            (TileRegion::RightHalf, Lang::En) => "Right Half",
            (TileRegion::TopHalf, Lang::En) => "Top Half",
            (TileRegion::BottomHalf, Lang::En) => "Bottom Half",
        }
    }

    fn apply(self, work_area: RECT) -> RECT {
        let w = work_area.right - work_area.left;
        let h = work_area.bottom - work_area.top;
        match self {
            TileRegion::FullWorkArea => work_area,
            TileRegion::LeftHalf => RECT {
                left: work_area.left,
                top: work_area.top,
                right: work_area.left + w / 2,
                bottom: work_area.bottom,
            },
            TileRegion::RightHalf => RECT {
                left: work_area.left + w / 2,
                top: work_area.top,
                right: work_area.right,
                bottom: work_area.bottom,
            },
            TileRegion::TopHalf => RECT {
                left: work_area.left,
                top: work_area.top,
                right: work_area.right,
                bottom: work_area.top + h / 2,
            },
            TileRegion::BottomHalf => RECT {
                left: work_area.left,
                top: work_area.top + h / 2,
                right: work_area.right,
                bottom: work_area.bottom,
            },
        }
    }
}

/// Tracks an active mouse move/resize operation on a grouped window
#[derive(Clone, Copy)]
struct DragState {
    hwnd: HWND,
    start_rect: RECT,
}

/// Window group state manager / 窗口组状态管理器
struct GroupManager {
    windows: Vec<HWND>,
    original_rects: HashMap<HWND, RECT>,
    /// Tracks previous `IsIconic` state of each grouped window to detect minimize/restore transitions reliably
    last_iconic: HashMap<HWND, bool>,
    /// Tracks whether the active foreground window was inside the group on the last tick
    last_fg_in_group: bool,
    region: TileRegion,
    target_monitor: Option<HMONITOR>,
    lang: Lang,
    suppress_sync_until: Option<Instant>,
    console_allocated: bool,
    msg_hwnd: HWND,
    active_drag: Option<DragState>,
    post_swap_focus: Option<HWND>,
}

impl GroupManager {
    fn new(lang: Lang) -> Self {
        Self {
            windows: Vec::new(),
            original_rects: HashMap::new(),
            last_iconic: HashMap::new(),
            last_fg_in_group: false,
            region: TileRegion::FullWorkArea,
            target_monitor: None,
            lang,
            suppress_sync_until: None,
            console_allocated: false,
            msg_hwnd: null_mut(),
            active_drag: None,
            post_swap_focus: None,
        }
    }

    fn mark_sync_cooldown(&mut self) {
        self.suppress_sync_until = Some(Instant::now() + Duration::from_millis(500));
    }

    fn is_in_sync_cooldown(&self) -> bool {
        self.suppress_sync_until
            .map(|until| Instant::now() < until)
            .unwrap_or(false)
    }

    fn sync_iconic_snapshot(&mut self) {
        self.last_iconic.clear();
        for &hwnd in &self.windows {
            let iconic = unsafe { IsIconic(hwnd) != 0 };
            self.last_iconic.insert(hwnd, iconic);
        }
        let fg = unsafe { GetForegroundWindow() };
        self.last_fg_in_group = !fg.is_null() && self.windows.contains(&fg);
    }

    fn toggle_language(&mut self) {
        self.lang = self.lang.toggle();
        print_banner(self.lang);
        self.print_group_status();
        self.update_tray_tooltip();
    }

    fn toggle_console(&mut self) {
        unsafe {
            if self.console_allocated {
                FreeConsole();
                self.console_allocated = false;
            } else if AllocConsole() != 0 {
                self.console_allocated = true;
                disable_console_quick_edit();
                print_banner(self.lang);
                self.print_group_status();
            }
        }
    }

    fn update_tray_tooltip(&self) {
        if self.msg_hwnd.is_null() {
            return;
        }
        let tip = match self.lang {
            Lang::Zh => format!(
                "TileGroup - 当前组内 {} 个窗口 (右键打开菜单)",
                self.windows.len()
            ),
            Lang::En => format!(
                "TileGroup - {} window(s) in group (Right-click for menu)",
                self.windows.len()
            ),
        };
        unsafe {
            let mut nid: NOTIFYICONDATAW = zeroed();
            nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = self.msg_hwnd;
            nid.uID = TRAY_ICON_ID;
            nid.uFlags = NIF_TIP;
            copy_wide_str(&mut nid.szTip, &tip);
            Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }

    /// Prune closed or invalid window handles
    fn prune_closed_windows(&mut self) -> bool {
        let before = self.windows.len();
        self.windows.retain(|&hwnd| unsafe { IsWindow(hwnd) != 0 });
        self.original_rects
            .retain(|&hwnd, _| unsafe { IsWindow(hwnd) != 0 });
        self.last_iconic
            .retain(|&hwnd, _| unsafe { IsWindow(hwnd) != 0 });
        if self.windows.is_empty() {
            self.target_monitor = None;
        }
        let changed = self.windows.len() != before;
        if changed {
            self.update_tray_tooltip();
        }
        changed
    }

    /// Add or remove the current foreground window
    fn toggle_foreground_window(&mut self) {
        self.prune_closed_windows();

        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.is_null() || !is_manageable_window(hwnd) {
            match self.lang {
                Lang::Zh => println!("⚠️ 当前激活窗口不可被编组平铺。"),
                Lang::En => println!("⚠️ The active foreground window cannot be tiled."),
            }
            return;
        }

        let title = get_window_title(hwnd);

        if let Some(pos) = self.windows.iter().position(|&w| w == hwnd) {
            self.windows.remove(pos);
            self.last_iconic.remove(&hwnd);
            if let Some(orig) = self.original_rects.remove(&hwnd) {
                self.mark_sync_cooldown();
                unsafe {
                    if IsZoomed(hwnd) != 0 || IsIconic(hwnd) != 0 {
                        restore_window_force(hwnd);
                    }
                    SetWindowPos(
                        hwnd,
                        null_mut(),
                        orig.left,
                        orig.top,
                        orig.right - orig.left,
                        orig.bottom - orig.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
            }
            match self.lang {
                Lang::Zh => println!(
                    "➖ 已移出窗口组: [{}] (当前组内剩余 {} 个窗口)",
                    title,
                    self.windows.len()
                ),
                Lang::En => println!(
                    "➖ Removed from group: [{}] ({} window(s) remaining)",
                    title,
                    self.windows.len()
                ),
            }
            self.print_group_status();
            self.retile();
            self.update_tray_tooltip();
        } else {
            let mut rect: RECT = unsafe { zeroed() };
            if unsafe { GetWindowRect(hwnd, &mut rect) } != 0 {
                self.original_rects.insert(hwnd, rect);
            }

            let mon = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
            if !mon.is_null() {
                self.target_monitor = Some(mon);
            }

            self.windows.push(hwnd);
            self.last_iconic.insert(hwnd, false);
            match self.lang {
                Lang::Zh => println!(
                    "➕ 已加入窗口组: [{}] (当前组内共 {} 个窗口)",
                    title,
                    self.windows.len()
                ),
                Lang::En => println!(
                    "➕ Added to group: [{}] ({} window(s) in group)",
                    title,
                    self.windows.len()
                ),
            }
            self.print_group_status();
            self.retile();
            self.update_tray_tooltip();
        }
    }

    /// Minimize or restore the entire group at once via hotkey or tray menu
    fn toggle_minimize_group(&mut self) {
        self.prune_closed_windows();
        if self.windows.is_empty() {
            match self.lang {
                Lang::Zh => println!("ℹ️ 当前窗口组为空，请先按 Alt+G 将窗口加入组。"),
                Lang::En => println!("ℹ️ Window group is empty. Press Alt+G to add windows first."),
            }
            return;
        }

        self.mark_sync_cooldown();

        let all_minimized = self
            .windows
            .iter()
            .all(|&hwnd| unsafe { IsIconic(hwnd) != 0 });

        if all_minimized {
            for &hwnd in &self.windows {
                unsafe {
                    restore_window_force(hwnd);
                }
            }
            self.retile();
            if let Some(&last_hwnd) = self.windows.last() {
                unsafe {
                    SetForegroundWindow(last_hwnd);
                }
            }
            self.sync_iconic_snapshot();
            match self.lang {
                Lang::Zh => println!("🔼 已一键还原整个窗口组 ({} 个窗口)", self.windows.len()),
                Lang::En => println!(
                    "🔼 Restored entire window group ({} window(s))",
                    self.windows.len()
                ),
            }
        } else {
            for &hwnd in &self.windows {
                unsafe {
                    ShowWindow(hwnd, SW_MINIMIZE);
                }
            }
            self.sync_iconic_snapshot();
            self.last_fg_in_group = false;
            match self.lang {
                Lang::Zh => println!("🔽 已一键最小化整个窗口组 ({} 个窗口)", self.windows.len()),
                Lang::En => println!(
                    "🔽 Minimized entire window group ({} window(s))",
                    self.windows.len()
                ),
            }
        }
    }

    /// Periodic ground-truth state check (runs every 150ms & on WinEvents):
    /// 1. If any grouped window transitioned from Minimized -> Open, restore & retile the entire group!
    /// 2. If any grouped window transitioned from Open -> Minimized, minimize the entire group!
    /// 3. If foreground focus switched from an outside window to a grouped window, bring the entire group to front!
    fn check_window_states(&mut self) {
        // Do not interfere while the user is actively dragging or resizing a grouped window
        if self.active_drag.is_some() {
            return;
        }

        if self.prune_closed_windows() && !self.windows.is_empty() {
            self.retile();
            return;
        }

        if self.windows.len() <= 1 || self.is_in_sync_cooldown() {
            return;
        }

        // 1. Check for Iconic (Minimize / Restore) state transitions
        let mut newly_restored: Option<HWND> = None;
        let mut newly_minimized: Option<HWND> = None;

        for &hwnd in &self.windows {
            let now_iconic = unsafe { IsIconic(hwnd) != 0 };
            let was_iconic = self.last_iconic.get(&hwnd).copied().unwrap_or(false);

            if was_iconic && !now_iconic {
                newly_restored = Some(hwnd);
                break;
            } else if !was_iconic && now_iconic {
                newly_minimized = Some(hwnd);
                break;
            }
        }

        if let Some(trigger_hwnd) = newly_restored {
            self.mark_sync_cooldown();
            for &hwnd in &self.windows {
                if hwnd != trigger_hwnd {
                    unsafe {
                        if IsIconic(hwnd) != 0 {
                            restore_window_force(hwnd);
                        }
                    }
                }
            }
            self.retile_Keeping_focus(trigger_hwnd);
            self.sync_iconic_snapshot();
            match self.lang {
                Lang::Zh => println!(
                    "🔗 检测到组内窗口 [{}] 唤醒，已联动还原并平铺整组 ({} 个窗口)",
                    get_window_title(trigger_hwnd),
                    self.windows.len()
                ),
                Lang::En => println!(
                    "🔗 [{}] restored -> linked-restored & tiled entire group ({} windows)",
                    get_window_title(trigger_hwnd),
                    self.windows.len()
                ),
            }
            return;
        }

        if let Some(trigger_hwnd) = newly_minimized {
            self.mark_sync_cooldown();
            for &hwnd in &self.windows {
                if hwnd != trigger_hwnd {
                    unsafe {
                        if IsIconic(hwnd) == 0 {
                            ShowWindowAsync(hwnd, SW_MINIMIZE);
                            ShowWindow(hwnd, SW_MINIMIZE);
                        }
                    }
                }
            }
            self.sync_iconic_snapshot();
            self.last_fg_in_group = false;
            match self.lang {
                Lang::Zh => println!(
                    "🔗 检测到组内窗口 [{}] 最小化，已联动最小化整组 ({} 个窗口)",
                    get_window_title(trigger_hwnd),
                    self.windows.len()
                ),
                Lang::En => println!(
                    "🔗 [{}] minimized -> linked-minimized entire group ({} windows)",
                    get_window_title(trigger_hwnd),
                    self.windows.len()
                ),
            }
            return;
        }

        // 2. Check if user clicked into a grouped window from an outside window (or while another member is minimized)
        let fg = unsafe { GetForegroundWindow() };
        let now_fg_in_group = !fg.is_null() && self.windows.contains(&fg);

        if now_fg_in_group && unsafe { IsIconic(fg) == 0 } {
            let any_other_minimized = self
                .windows
                .iter()
                .any(|&w| w != fg && unsafe { IsIconic(w) != 0 });

            if !self.last_fg_in_group || any_other_minimized {
                self.mark_sync_cooldown();
                self.retile_Keeping_focus(fg);
                self.sync_iconic_snapshot();
            }
        }

        self.last_fg_in_group = now_fg_in_group;
    }

    /// Move the active grouped window backward (`delta = -1`) or forward (`delta = 1`) in the layout slots
    fn move_active_window(&mut self, delta: isize) {
        self.prune_closed_windows();
        let n = self.windows.len();
        if n <= 1 {
            return;
        }

        let fg = unsafe { GetForegroundWindow() };
        let Some(idx) = self.windows.iter().position(|&w| w == fg) else {
            match self.lang {
                Lang::Zh => println!("ℹ️ 当前激活窗口不在窗口组内，无法调整位置。"),
                Lang::En => println!("ℹ️ Active window is not in the group."),
            }
            return;
        };

        let new_idx = (idx as isize + delta).rem_euclid(n as isize) as usize;
        self.windows.swap(idx, new_idx);
        match self.lang {
            Lang::Zh => println!(
                "🔀 调整窗口槽位: [{}] (#{} <-> #{})",
                get_window_title(fg),
                idx + 1,
                new_idx + 1
            ),
            Lang::En => println!(
                "🔀 Swapped window slot: [{}] (#{} <-> #{})",
                get_window_title(fg),
                idx + 1,
                new_idx + 1
            ),
        }
        self.print_group_status();
        self.retile_Keeping_focus(fg);
    }

    /// Record initial window bounds when the user starts dragging or resizing a grouped window
    fn on_move_size_start(&mut self, hwnd: HWND) {
        if !self.windows.contains(&hwnd) {
            return;
        }
        let mut rect: RECT = unsafe { zeroed() };
        if unsafe { GetWindowRect(hwnd, &mut rect) } != 0 {
            self.active_drag = Some(DragState {
                hwnd,
                start_rect: rect,
            });
        }
    }

    /// When user finishes dragging a grouped window, swap it with the target grid cell under the cursor
    fn on_move_size_end(&mut self, hwnd: HWND) {
        let drag = self.active_drag.take();
        self.prune_closed_windows();
        let n = self.windows.len();
        if n <= 1 {
            return;
        }

        let Some(src_idx) = self.windows.iter().position(|&w| w == hwnd) else {
            return;
        };

        // Distinguish border/corner resize from title-bar drag:
        // If any corner remained anchored while width/height changed, the user manually resized an edge -> keep it!
        if let Some(d) = drag.filter(|d| d.hwnd == hwnd) {
            let mut end_rect: RECT = unsafe { zeroed() };
            if unsafe { GetWindowRect(hwnd, &mut end_rect) } != 0 {
                let start_w = d.start_rect.right - d.start_rect.left;
                let start_h = d.start_rect.bottom - d.start_rect.top;
                let end_w = end_rect.right - end_rect.left;
                let end_h = end_rect.bottom - end_rect.top;

                let same_tl = (end_rect.left - d.start_rect.left).abs() <= 3
                    && (end_rect.top - d.start_rect.top).abs() <= 3;
                let same_br = (end_rect.right - d.start_rect.right).abs() <= 3
                    && (end_rect.bottom - d.start_rect.bottom).abs() <= 3;
                let same_bl = (end_rect.left - d.start_rect.left).abs() <= 3
                    && (end_rect.bottom - d.start_rect.bottom).abs() <= 3;
                let same_tr = (end_rect.right - d.start_rect.right).abs() <= 3
                    && (end_rect.top - d.start_rect.top).abs() <= 3;

                let size_changed = (end_w - start_w).abs() > 6 || (end_h - start_h).abs() > 6;
                if (same_tl || same_br || same_bl || same_tr) && size_changed {
                    return;
                }
            }
        }

        let mut cursor: POINT = unsafe { zeroed() };
        if unsafe { GetCursorPos(&mut cursor) } == 0 {
            return;
        }

        let work_area = self.get_target_work_area();
        let bounds = self.region.apply(work_area);
        let target_rects = calculate_grid_layout(bounds, n);

        let dst_idx = target_rects.iter().position(|r| {
            cursor.x >= r.left && cursor.x < r.right && cursor.y >= r.top && cursor.y < r.bottom
        });

        if let Some(dst) = dst_idx {
            if dst != src_idx {
                self.windows.swap(src_idx, dst);
                match self.lang {
                    Lang::Zh => println!(
                        "🖱️ 拖拽对调窗口位置: #{} <-> #{} ([{}])",
                        src_idx + 1,
                        dst + 1,
                        get_window_title(hwnd)
                    ),
                    Lang::En => println!(
                        "🖱️ Drag-swapped slots: #{} <-> #{} ([{}])",
                        src_idx + 1,
                        dst + 1,
                        get_window_title(hwnd)
                    ),
                }
                self.print_group_status();
            }
            // Snap into target slot (and schedule a 70ms follow-up in case Windows Aero Snap triggers on screen edges)
            self.retile_Keeping_focus(hwnd);
            if !self.msg_hwnd.is_null() {
                self.post_swap_focus = Some(hwnd);
                unsafe {
                    SetTimer(self.msg_hwnd, TIMER_POST_SWAP, 70, None);
                }
            }
        }
    }

    /// Cycle through tiling regions and immediately re-tile
    fn cycle_region(&mut self) {
        self.region = self.region.next();
        match self.lang {
            Lang::Zh => println!("📐 切换平铺区域为: {}", self.region.name(self.lang)),
            Lang::En => println!("📐 Switched tiling region to: {}", self.region.name(self.lang)),
        }
        self.retile();
    }

    /// Clear the window group
    fn clear_group(&mut self) {
        self.windows.clear();
        self.original_rects.clear();
        self.last_iconic.clear();
        self.last_fg_in_group = false;
        self.target_monitor = None;
        self.update_tray_tooltip();
        match self.lang {
            Lang::Zh => println!("🧹 已清空窗口组，所有窗口恢复自由状态。"),
            Lang::En => println!("🧹 Cleared window group. All windows are now unmanaged."),
        }
    }

    fn retile(&mut self) {
        let fg = unsafe { GetForegroundWindow() };
        self.retile_Keeping_focus(fg);
    }

    /// Dynamically divide target region, restore any minimized members, and bring all grouped windows to front
    #[allow(non_snake_case)]
    fn retile_Keeping_focus(&mut self, preferred_fg: HWND) {
        self.prune_closed_windows();
        let n = self.windows.len();
        if n == 0 {
            return;
        }

        self.mark_sync_cooldown();

        let work_area = self.get_target_work_area();
        let bounds = self.region.apply(work_area);
        let target_rects = calculate_grid_layout(bounds, n);

        for (&hwnd, target) in self.windows.iter().zip(target_rects.iter()) {
            unsafe {
                if IsIconic(hwnd) != 0 || IsZoomed(hwnd) != 0 {
                    restore_window_force(hwnd);
                }
                set_window_pos_compensated(hwnd, *target);
            }
        }

        if !preferred_fg.is_null() && self.windows.contains(&preferred_fg) {
            unsafe {
                keybd_event(0, 0, KEYEVENTF_KEYUP, 0);
                SetForegroundWindow(preferred_fg);
            }
        }

        self.sync_iconic_snapshot();
    }

    /// Get work area of the target monitor (excluding taskbar)
    fn get_target_work_area(&self) -> RECT {
        unsafe {
            let monitor = self
                .target_monitor
                .or_else(|| {
                    self.windows
                        .first()
                        .map(|&hwnd| MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST))
                })
                .unwrap_or_else(|| {
                    let mut pt: POINT = zeroed();
                    GetCursorPos(&mut pt);
                    MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST)
                });

            let mut mi: MONITORINFO = zeroed();
            mi.cbSize = size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(monitor, &mut mi) != 0 {
                mi.rcWork
            } else {
                RECT {
                    left: 0,
                    top: 0,
                    right: 1920,
                    bottom: 1080,
                }
            }
        }
    }

    fn print_group_status(&self) {
        if self.windows.is_empty() {
            match self.lang {
                Lang::Zh => println!("   [当前窗口组为空]"),
                Lang::En => println!("   [Group is currently empty]"),
            }
            return;
        }
        for (idx, &hwnd) in self.windows.iter().enumerate() {
            println!("   {}. {}", idx + 1, get_window_title(hwnd));
        }
    }
}

/// Reliably restore a minimized or maximized window even when called from a background timer/hook
unsafe fn restore_window_force(hwnd: HWND) {
    // Unlock Windows ForegroundLockTimeout using dummy keyup + AttachThreadInput
    keybd_event(0, 0, KEYEVENTF_KEYUP, 0);

    let fg = GetForegroundWindow();
    let cur_tid = GetCurrentThreadId();
    let fg_tid = if !fg.is_null() {
        GetWindowThreadProcessId(fg, null_mut())
    } else {
        0
    };
    let target_tid = GetWindowThreadProcessId(hwnd, null_mut());

    if fg_tid != 0 && fg_tid != cur_tid {
        AttachThreadInput(cur_tid, fg_tid, 1);
    }
    if target_tid != 0 && target_tid != cur_tid {
        AttachThreadInput(cur_tid, target_tid, 1);
    }

    ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    ShowWindow(hwnd, SW_RESTORE);

    if IsIconic(hwnd) != 0 {
        SendMessageTimeoutW(
            hwnd,
            WM_SYSCOMMAND,
            SC_RESTORE as usize,
            0,
            SMTO_ABORTIFHUNG,
            100,
            null_mut(),
        );
    }

    if target_tid != 0 && target_tid != cur_tid {
        AttachThreadInput(cur_tid, target_tid, 0);
    }
    if fg_tid != 0 && fg_tid != cur_tid {
        AttachThreadInput(cur_tid, fg_tid, 0);
    }
}

/// Calculate grid layout rectangles for `n` windows inside `bounds`
fn calculate_grid_layout(bounds: RECT, n: usize) -> Vec<RECT> {
    if n == 0 {
        return Vec::new();
    }

    let total_w = bounds.right - bounds.left;
    let total_h = bounds.bottom - bounds.top;

    let rows = match n {
        1..=3 => 1,
        4 => 2,
        _ => (n as f64).sqrt().floor() as usize,
    };

    let mut rects = Vec::with_capacity(n);
    let base_cols = n / rows;
    let extra = n % rows;

    for r in 0..rows {
        // Place larger cells (fewer columns) on the top rows, and smaller cells on the bottom rows
        // e.g. for n = 5 (rows = 2): top row has 2 big slots, bottom row has 3 smaller slots
        let cols_in_row = if r >= rows - extra {
            base_cols + 1
        } else {
            base_cols
        };
        let top = bounds.top + (total_h * r as i32) / rows as i32;
        let bottom = bounds.top + (total_h * (r + 1) as i32) / rows as i32;

        for c in 0..cols_in_row {
            let left = bounds.left + (total_w * c as i32) / cols_in_row as i32;
            let right = bounds.left + (total_w * (c + 1) as i32) / cols_in_row as i32;
            rects.push(RECT {
                left,
                top,
                right,
                bottom,
            });
        }
    }

    rects
}

/// Compensate for Windows 10/11 DWM invisible borders (~7px) and bring window to front of Z-order
unsafe fn set_window_pos_compensated(hwnd: HWND, target: RECT) {
    // Use TOPMOST -> NOTOPMOST promotion so non-active grouped windows are guaranteed to rise above covering windows
    SetWindowPos(
        hwnd,
        HWND_TOPMOST,
        target.left,
        target.top,
        target.right - target.left,
        target.bottom - target.top,
        SWP_NOACTIVATE,
    );

    let mut win_rect: RECT = zeroed();
    let mut frame_rect: RECT = zeroed();

    let got_win = GetWindowRect(hwnd, &mut win_rect) != 0;
    let got_frame = DwmGetWindowAttribute(
        hwnd,
        DWMWA_EXTENDED_FRAME_BOUNDS as u32,
        &mut frame_rect as *mut _ as *mut _,
        size_of::<RECT>() as u32,
    ) == S_OK;

    if got_win && got_frame {
        let left_margin = (frame_rect.left - win_rect.left).clamp(0, 40);
        let top_margin = (frame_rect.top - win_rect.top).clamp(0, 40);
        let right_margin = (win_rect.right - frame_rect.right).clamp(0, 40);
        let bottom_margin = (win_rect.bottom - frame_rect.bottom).clamp(0, 40);

        let adj_left = target.left - left_margin;
        let adj_top = target.top - top_margin;
        let adj_width = (target.right - target.left) + left_margin + right_margin;
        let adj_height = (target.bottom - target.top) + top_margin + bottom_margin;

        SetWindowPos(
            hwnd,
            HWND_NOTOPMOST,
            adj_left,
            adj_top,
            adj_width,
            adj_height,
            SWP_NOACTIVATE,
        );
    } else {
        SetWindowPos(
            hwnd,
            HWND_NOTOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

/// Check if a window is a normal, resizable top-level application window
fn is_manageable_window(hwnd: HWND) -> bool {
    unsafe {
        if IsWindow(hwnd) == 0 || IsWindowVisible(hwnd) == 0 {
            return false;
        }
        if hwnd == GetShellWindow() {
            return false;
        }
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if (ex_style & WS_EX_TOOLWINDOW) != 0 {
            return false;
        }
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        if (style & WS_THICKFRAME) == 0 {
            return false;
        }
        true
    }
}

/// Get window title string for logging
fn get_window_title(hwnd: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return format!("HWND({:?})", hwnd);
        }
        let mut buf = vec![0u16; (len + 1) as usize];
        let read = GetWindowTextW(hwnd, buf.as_mut_ptr(), len + 1);
        String::from_utf16_lossy(&buf[..read as usize])
    }
}

fn to_wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn copy_wide_str(dest: &mut [u16], src: &str) {
    let encoded: Vec<u16> = src.encode_utf16().collect();
    let max_len = dest.len().saturating_sub(1);
    let copy_len = encoded.len().min(max_len);
    dest[..copy_len].copy_from_slice(&encoded[..copy_len]);
    dest[copy_len] = 0;
}

thread_local! {
    static MANAGER: RefCell<GroupManager> = RefCell::new(GroupManager::new(Lang::detect()));
}

/// Listen for system window events and trigger immediate state check
unsafe extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _id_event_thread: u32,
    _dwms_event_time: u32,
) {
    if id_object != OBJID_WINDOW || id_child != 0 || hwnd.is_null() {
        return;
    }

    match event {
        EVENT_OBJECT_DESTROY => {
            let really_closed =
                IsWindow(hwnd) == 0 || (IsWindowVisible(hwnd) == 0 && IsIconic(hwnd) == 0);
            if !really_closed {
                return;
            }

            MANAGER.with(|mgr| {
                let mut mgr = mgr.borrow_mut();
                if let Some(pos) = mgr.windows.iter().position(|&w| w == hwnd) {
                    mgr.windows.remove(pos);
                    mgr.original_rects.remove(&hwnd);
                    mgr.last_iconic.remove(&hwnd);
                    match mgr.lang {
                        Lang::Zh => println!(
                            "🔄 检测到组内窗口关闭，自动重排剩余 {} 个窗口",
                            mgr.windows.len()
                        ),
                        Lang::En => println!(
                            "🔄 Grouped window closed; auto-retiling {} remaining window(s)",
                            mgr.windows.len()
                        ),
                    }
                    mgr.retile();
                    mgr.update_tray_tooltip();
                }
            });
        }
        EVENT_SYSTEM_MINIMIZESTART | EVENT_SYSTEM_MINIMIZEEND | EVENT_SYSTEM_FOREGROUND => {
            MANAGER.with(|mgr| mgr.borrow_mut().check_window_states());
        }
        EVENT_SYSTEM_MOVESIZESTART => {
            MANAGER.with(|mgr| mgr.borrow_mut().on_move_size_start(hwnd));
        }
        EVENT_SYSTEM_MOVESIZEEND => {
            MANAGER.with(|mgr| mgr.borrow_mut().on_move_size_end(hwnd));
        }
        _ => {}
    }
}

/// Show native right-click context menu for the System Tray icon
unsafe fn show_tray_context_menu(hwnd: HWND) {
    let (lang, count, has_console) = MANAGER.with(|m| {
        let mgr = m.borrow();
        (mgr.lang, mgr.windows.len(), mgr.console_allocated)
    });

    let hmenu = CreatePopupMenu();
    if hmenu.is_null() {
        return;
    }

    let (txt_retile, txt_hide, txt_region, txt_clear, txt_console, txt_lang, txt_quit) = match lang {
        Lang::Zh => (
            format!("🔄 重新平铺当前组 ({} 个窗口)\tAlt+T", count),
            "🔽 隐藏 / 还原整个窗口组\tAlt+H".to_string(),
            "📐 切换平铺区域\tAlt+S".to_string(),
            "🧹 清空窗口组\tAlt+C".to_string(),
            if has_console {
                "🖥️ 隐藏日志终端窗口".to_string()
            } else {
                "🖥️ 显示日志终端窗口".to_string()
            },
            "🌐 切换语言 (中文 / EN)\tAlt+L".to_string(),
            "❌ 退出 TileGroup\tAlt+Q".to_string(),
        ),
        Lang::En => (
            format!("🔄 Re-tile Group ({} windows)\tAlt+T", count),
            "🔽 Hide / Restore Group\tAlt+H".to_string(),
            "📐 Switch Tiling Region\tAlt+S".to_string(),
            "🧹 Clear Window Group\tAlt+C".to_string(),
            if has_console {
                "🖥️ Hide Log Console".to_string()
            } else {
                "🖥️ Show Log Console".to_string()
            },
            "🌐 Switch Language (EN / 中文)\tAlt+L".to_string(),
            "❌ Quit TileGroup\tAlt+Q".to_string(),
        ),
    };

    AppendMenuW(hmenu, MF_STRING, CMD_RETILE, to_wide_null(&txt_retile).as_ptr());
    AppendMenuW(hmenu, MF_STRING, CMD_MIN_RESTORE, to_wide_null(&txt_hide).as_ptr());
    AppendMenuW(hmenu, MF_STRING, CMD_CYCLE_REGION, to_wide_null(&txt_region).as_ptr());
    AppendMenuW(hmenu, MF_STRING, CMD_CLEAR_GROUP, to_wide_null(&txt_clear).as_ptr());
    AppendMenuW(hmenu, MF_SEPARATOR, 0, null_mut());
    AppendMenuW(hmenu, MF_STRING, CMD_TOGGLE_CONSOLE, to_wide_null(&txt_console).as_ptr());
    AppendMenuW(hmenu, MF_STRING, CMD_TOGGLE_LANG, to_wide_null(&txt_lang).as_ptr());
    AppendMenuW(hmenu, MF_SEPARATOR, 0, null_mut());
    AppendMenuW(hmenu, MF_STRING, CMD_QUIT, to_wide_null(&txt_quit).as_ptr());

    let mut pt: POINT = zeroed();
    GetCursorPos(&mut pt);
    SetForegroundWindow(hwnd);
    TrackPopupMenu(
        hmenu,
        TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RIGHTBUTTON,
        pt.x,
        pt.y,
        0,
        hwnd,
        null_mut(),
    );
    DestroyMenu(hmenu);
}

/// Hidden window procedure handling Tray Icon events, Menu Commands, and Polling Timer
unsafe extern "system" fn tray_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TRAYICON => {
            let event = (lparam & 0xFFFF) as u32;
            if event == WM_RBUTTONUP {
                show_tray_context_menu(hwnd);
            } else if event == WM_LBUTTONDBLCLK {
                MANAGER.with(|m| m.borrow_mut().toggle_minimize_group());
            }
            0
        }
        WM_COMMAND => {
            let cmd = wparam & 0xFFFF;
            match cmd {
                CMD_RETILE => MANAGER.with(|m| m.borrow_mut().retile()),
                CMD_MIN_RESTORE => MANAGER.with(|m| m.borrow_mut().toggle_minimize_group()),
                CMD_CYCLE_REGION => MANAGER.with(|m| m.borrow_mut().cycle_region()),
                CMD_CLEAR_GROUP => MANAGER.with(|m| m.borrow_mut().clear_group()),
                CMD_TOGGLE_CONSOLE => MANAGER.with(|m| m.borrow_mut().toggle_console()),
                CMD_TOGGLE_LANG => MANAGER.with(|m| m.borrow_mut().toggle_language()),
                CMD_QUIT => PostQuitMessage(0),
                _ => {}
            }
            0
        }
        WM_TIMER => {
            if wparam == TIMER_STATE_POLL {
                MANAGER.with(|m| m.borrow_mut().check_window_states());
            } else if wparam == TIMER_POST_SWAP {
                KillTimer(hwnd, TIMER_POST_SWAP);
                MANAGER.with(|m| {
                    let mut mgr = m.borrow_mut();
                    let focus = mgr
                        .post_swap_focus
                        .take()
                        .unwrap_or_else(|| GetForegroundWindow());
                    mgr.retile_Keeping_focus(focus);
                });
            }
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn create_tray_window_and_icon(lang: Lang) -> HWND {
    let class_name = to_wide_null("TileGroupTrayClass");
    let hinstance = GetModuleHandleW(null_mut());

    let mut wc: WNDCLASSW = zeroed();
    wc.lpfnWndProc = Some(tray_wnd_proc);
    wc.hInstance = hinstance;
    wc.lpszClassName = class_name.as_ptr();
    RegisterClassW(&wc);

    let hwnd = CreateWindowExW(
        0,
        class_name.as_ptr(),
        to_wide_null("TileGroup").as_ptr(),
        WS_OVERLAPPEDWINDOW,
        0,
        0,
        0,
        0,
        null_mut(),
        null_mut(),
        hinstance,
        null_mut(),
    );

    if !hwnd.is_null() {
        let mut nid: NOTIFYICONDATAW = zeroed();
        nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = TRAY_ICON_ID;
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_INFO;
        nid.uCallbackMessage = WM_TRAYICON;
        nid.hIcon = LoadIconW(null_mut(), IDI_APPLICATION);
        nid.dwInfoFlags = NIIF_INFO;

        match lang {
            Lang::Zh => {
                copy_wide_str(&mut nid.szTip, "TileGroup - 轻量级窗口组平铺工具 (右键菜单)");
                copy_wide_str(&mut nid.szInfoTitle, "TileGroup 已在后台运行");
                copy_wide_str(
                    &mut nid.szInfo,
                    "Alt+G: 加入/移出窗口组 | Alt+H: 隐藏/还原整组 | 右键托盘图标查看更多",
                );
            }
            Lang::En => {
                copy_wide_str(&mut nid.szTip, "TileGroup - Window Group Tiler (Right-click)");
                copy_wide_str(&mut nid.szInfoTitle, "TileGroup is running in System Tray");
                copy_wide_str(
                    &mut nid.szInfo,
                    "Alt+G: Add/Remove Window | Alt+H: Hide/Restore Group | Right-click tray icon",
                );
            }
        }

        Shell_NotifyIconW(NIM_ADD, &nid);
    }

    hwnd
}

unsafe fn remove_tray_icon(hwnd: HWND) {
    if !hwnd.is_null() {
        let mut nid: NOTIFYICONDATAW = zeroed();
        nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = TRAY_ICON_ID;
        Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

/// Disable Console QuickEdit Mode so clicking inside the terminal never freezes the process
fn disable_console_quick_edit() {
    unsafe {
        let h_in = GetStdHandle(STD_INPUT_HANDLE);
        if !h_in.is_null() {
            let mut mode: u32 = 0;
            if GetConsoleMode(h_in, &mut mode) != 0 {
                mode &= !ENABLE_QUICK_EDIT_MODE;
                mode |= ENABLE_EXTENDED_FLAGS;
                SetConsoleMode(h_in, mode);
            }
        }
    }
}

fn register_hotkeys(lang: Lang) -> bool {
    unsafe {
        let mods = MOD_ALT | MOD_NOREPEAT;
        let ok_g = RegisterHotKey(null_mut(), HK_TOGGLE_WINDOW, mods, b'G' as u32) != 0;
        let ok_h = RegisterHotKey(null_mut(), HK_MIN_RESTORE, mods, b'H' as u32) != 0;
        let ok_s = RegisterHotKey(null_mut(), HK_CYCLE_REGION, mods, b'S' as u32) != 0;
        let ok_t = RegisterHotKey(null_mut(), HK_RETILE, mods, b'T' as u32) != 0;
        let ok_c = RegisterHotKey(null_mut(), HK_CLEAR_GROUP, mods, b'C' as u32) != 0;
        let ok_l = RegisterHotKey(null_mut(), HK_TOGGLE_LANG, mods, b'L' as u32) != 0;
        let ok_q = RegisterHotKey(null_mut(), HK_QUIT, mods, b'Q' as u32) != 0;
        let ok_prev = RegisterHotKey(null_mut(), HK_MOVE_PREV, mods, VK_OEM_4 as u32) != 0;
        let ok_next = RegisterHotKey(null_mut(), HK_MOVE_NEXT, mods, VK_OEM_6 as u32) != 0;

        if !(ok_g && ok_h && ok_s && ok_t && ok_c && ok_l && ok_q && ok_prev && ok_next) {
            match lang {
                Lang::Zh => eprintln!("⚠️ 部分快捷键注册失败（可能与其他软件冲突）:"),
                Lang::En => eprintln!("⚠️ Failed to register some hotkeys (possible conflict):"),
            }
            eprintln!(
                "   Alt+G: {}, Alt+H: {}, Alt+[: {}, Alt+]: {}, Alt+S: {}, Alt+T: {}, Alt+C: {}, Alt+L: {}, Alt+Q: {}",
                ok_g, ok_h, ok_prev, ok_next, ok_s, ok_t, ok_c, ok_l, ok_q
            );
            return false;
        }
        true
    }
}

fn unregister_hotkeys() {
    unsafe {
        for id in [
            HK_TOGGLE_WINDOW,
            HK_MIN_RESTORE,
            HK_CYCLE_REGION,
            HK_RETILE,
            HK_CLEAR_GROUP,
            HK_TOGGLE_LANG,
            HK_QUIT,
            HK_MOVE_PREV,
            HK_MOVE_NEXT,
        ] {
            UnregisterHotKey(null_mut(), id);
        }
    }
}

fn print_banner(lang: Lang) {
    match lang {
        Lang::Zh => {
            println!("==============================================================");
            println!(" 🪟 TileGroup v0.3.0 - 轻量级按需窗口组平铺工具 (系统托盘版)");
            println!("==============================================================");
            println!(" 快捷键说明：");
            println!("   [Alt + G]      将当前激活窗口 加入 / 移出 窗口组（自动均分平铺）");
            println!("   [Alt + [ / ]]  将当前窗口在组内 前移 / 后移 槽位（调换大小格位置）");
            println!("   [鼠标拖拽]      按住窗口标题栏拖到目标格子松手，自动与目标窗口对调位置");
            println!("   [Alt + H]      一键 隐藏(最小化) / 还原 整个窗口组");
            println!("   [Alt + S]      切换平铺区域（全屏 -> 左半屏 -> 右半屏 -> 上半屏 -> 下半屏）");
            println!("   [Alt + T]      手动重新对齐平铺当前组");
            println!("   [Alt + C]      清空窗口组（解散编组）");
            println!("   [Alt + L]      切换界面语言 (中文 / English)");
            println!("   [Alt + Q]      退出 TileGroup");
            println!("--------------------------------------------------------------");
            println!(" 💡 提示：5窗口等非对称布局下，前排槽位默认占大格；支持拖拽或快捷键自由换位！");
            println!("==============================================================\n");
        }
        Lang::En => {
            println!("==============================================================");
            println!(" 🪟 TileGroup v0.3.0 - Opt-in Window Group Tiler (System Tray)");
            println!("==============================================================");
            println!(" Hotkeys & Mouse:");
            println!("   [Alt + G]      Add / Remove active window to/from group & auto-tile");
            println!("   [Alt + [ / ]]  Move active window to Previous / Next slot in group");
            println!("   [Mouse Drag]   Drag a grouped window's title bar onto another slot to swap");
            println!("   [Alt + H]      Hide (Minimize) / Restore the entire window group");
            println!("   [Alt + S]      Switch tiling region (Full -> Left -> Right -> Top -> Bottom)");
            println!("   [Alt + T]      Re-tile current group manually");
            println!("   [Alt + C]      Clear window group (ungroup all)");
            println!("   [Alt + L]      Switch UI language (English / 中文)");
            println!("   [Alt + Q]      Quit TileGroup");
            println!("--------------------------------------------------------------");
            println!(" 💡 Tip: Front slots get larger cells (e.g. 5 windows = 2 big top + 3 bottom).");
            println!("==============================================================\n");
        }
    }
}

fn main() {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    // Optional `--console` flag if user wants the log console open at startup
    let want_console = env::args().any(|a| a == "--console" || a == "-c");
    let lang = MANAGER.with(|m| m.borrow().lang);

    if want_console {
        MANAGER.with(|m| m.borrow_mut().toggle_console());
    }

    register_hotkeys(lang);

    let msg_hwnd = unsafe { create_tray_window_and_icon(lang) };
    MANAGER.with(|m| m.borrow_mut().msg_hwnd = msg_hwnd);

    // Start a 150ms polling timer on the hidden tray window to reliably track Iconic & Foreground transitions
    if !msg_hwnd.is_null() {
        unsafe {
            SetTimer(msg_hwnd, TIMER_STATE_POLL, 150, None);
        }
    }

    let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;

    let hook_destroy = unsafe {
        SetWinEventHook(
            EVENT_OBJECT_DESTROY,
            EVENT_OBJECT_DESTROY,
            null_mut(),
            Some(win_event_proc),
            0,
            0,
            flags,
        )
    };

    let hook_minmax = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_MINIMIZESTART,
            EVENT_SYSTEM_MINIMIZEEND,
            null_mut(),
            Some(win_event_proc),
            0,
            0,
            flags,
        )
    };

    let hook_fg = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            null_mut(),
            Some(win_event_proc),
            0,
            0,
            flags,
        )
    };

    let hook_movesize = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_MOVESIZESTART,
            EVENT_SYSTEM_MOVESIZEEND,
            null_mut(),
            Some(win_event_proc),
            0,
            0,
            flags,
        )
    };

    unsafe {
        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            if msg.message == WM_HOTKEY {
                let id = msg.wParam as i32;
                match id {
                    HK_TOGGLE_WINDOW => MANAGER.with(|m| m.borrow_mut().toggle_foreground_window()),
                    HK_MIN_RESTORE => MANAGER.with(|m| m.borrow_mut().toggle_minimize_group()),
                    HK_CYCLE_REGION => MANAGER.with(|m| m.borrow_mut().cycle_region()),
                    HK_MOVE_PREV => MANAGER.with(|m| m.borrow_mut().move_active_window(-1)),
                    HK_MOVE_NEXT => MANAGER.with(|m| m.borrow_mut().move_active_window(1)),
                    HK_RETILE => {
                        MANAGER.with(|m| {
                            let mut mgr = m.borrow_mut();
                            match mgr.lang {
                                Lang::Zh => println!("🔄 手动触发重新平铺"),
                                Lang::En => println!("🔄 Manually re-tiling group"),
                            }
                            mgr.retile();
                        });
                    }
                    HK_CLEAR_GROUP => MANAGER.with(|m| m.borrow_mut().clear_group()),
                    HK_TOGGLE_LANG => MANAGER.with(|m| m.borrow_mut().toggle_language()),
                    HK_QUIT => {
                        PostQuitMessage(0);
                    }
                    _ => {}
                }
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        if !msg_hwnd.is_null() {
            KillTimer(msg_hwnd, TIMER_STATE_POLL);
            KillTimer(msg_hwnd, TIMER_POST_SWAP);
            remove_tray_icon(msg_hwnd);
            DestroyWindow(msg_hwnd);
        }
        if !hook_movesize.is_null() {
            UnhookWinEvent(hook_movesize);
        }
        if !hook_fg.is_null() {
            UnhookWinEvent(hook_fg);
        }
        if !hook_minmax.is_null() {
            UnhookWinEvent(hook_minmax);
        }
        if !hook_destroy.is_null() {
            UnhookWinEvent(hook_destroy);
        }
    }

    unregister_hotkeys();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grid_layout_1_to_4() {
        let bounds = RECT {
            left: 0,
            top: 0,
            right: 1200,
            bottom: 800,
        };

        let r1 = calculate_grid_layout(bounds, 1);
        assert_eq!(r1.len(), 1);
        assert_eq!((r1[0].left, r1[0].top, r1[0].right, r1[0].bottom), (0, 0, 1200, 800));

        let r2 = calculate_grid_layout(bounds, 2);
        assert_eq!(r2.len(), 2);
        assert_eq!((r2[0].left, r2[0].right), (0, 600));
        assert_eq!((r2[1].left, r2[1].right), (600, 1200));

        let r3 = calculate_grid_layout(bounds, 3);
        assert_eq!(r3.len(), 3);
        assert_eq!((r3[0].left, r3[0].right), (0, 400));
        assert_eq!((r3[1].left, r3[1].right), (400, 800));
        assert_eq!((r3[2].left, r3[2].right), (800, 1200));

        let r4 = calculate_grid_layout(bounds, 4);
        assert_eq!(r4.len(), 4);
        assert_eq!((r4[0].left, r4[0].top, r4[0].right, r4[0].bottom), (0, 0, 600, 400));
        assert_eq!((r4[1].left, r4[1].top, r4[1].right, r4[1].bottom), (600, 0, 1200, 400));
        assert_eq!((r4[2].left, r4[2].top, r4[2].right, r4[2].bottom), (0, 400, 600, 800));
        assert_eq!((r4[3].left, r4[3].top, r4[3].right, r4[3].bottom), (600, 400, 1200, 800));

        // N = 5: Top row has 2 BIG windows (600px wide), bottom row has 3 SMALL windows (400px wide)
        let r5 = calculate_grid_layout(bounds, 5);
        assert_eq!(r5.len(), 5);
        assert_eq!((r5[0].left, r5[0].top, r5[0].right, r5[0].bottom), (0, 0, 600, 400));
        assert_eq!((r5[1].left, r5[1].top, r5[1].right, r5[1].bottom), (600, 0, 1200, 400));
        assert_eq!((r5[2].left, r5[2].top, r5[2].right, r5[2].bottom), (0, 400, 400, 800));
        assert_eq!((r5[3].left, r5[3].top, r5[3].right, r5[3].bottom), (400, 400, 800, 800));
        assert_eq!((r5[4].left, r5[4].top, r5[4].right, r5[4].bottom), (800, 400, 1200, 800));
    }
}
