use std::cell::RefCell;
use std::collections::HashMap;
use std::env;
use std::mem::{size_of, zeroed};
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{HWND, POINT, RECT, S_OK};
use windows_sys::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, HMONITOR, MONITORINFO,
    MONITOR_DEFAULTTONEAREST,
};
use windows_sys::Win32::System::Console::{
    GetConsoleMode, GetStdHandle, SetConsoleMode, ENABLE_EXTENDED_FLAGS, ENABLE_QUICK_EDIT_MODE,
    STD_INPUT_HANDLE,
};
use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows_sys::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_NOREPEAT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetCursorPos, GetForegroundWindow, GetMessageW, GetShellWindow,
    GetWindowLongW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW, IsIconic, IsWindow,
    IsWindowVisible, IsZoomed, PostQuitMessage, SendMessageTimeoutW, SetForegroundWindow,
    SetWindowPos, ShowWindow, TranslateMessage, EVENT_OBJECT_DESTROY, GWL_EXSTYLE, GWL_STYLE,
    HWND_TOP, MSG, OBJID_WINDOW, SC_RESTORE, SMTO_ABORTIFHUNG, SWP_NOACTIVATE, SWP_NOZORDER,
    SW_MINIMIZE, SW_RESTORE, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_HOTKEY,
    WM_SYSCOMMAND, WS_EX_TOOLWINDOW, WS_THICKFRAME,
};

#[link(name = "kernel32")]
extern "system" {
    fn GetUserDefaultUILanguage() -> u16;
}

// Hotkey IDs / 快捷键 ID 定义
const HK_TOGGLE_WINDOW: i32 = 1; // Alt + G: Add / Remove current window & auto-tile
const HK_MIN_RESTORE: i32 = 2;   // Alt + H: Hide (Minimize) / Restore entire group
const HK_CYCLE_REGION: i32 = 3;  // Alt + S: Cycle tiling region
const HK_RETILE: i32 = 4;        // Alt + T: Manually re-tile current group
const HK_CLEAR_GROUP: i32 = 5;   // Alt + C: Clear window group
const HK_QUIT: i32 = 6;          // Alt + Q: Quit program
const HK_TOGGLE_LANG: i32 = 7;   // Alt + L: Switch UI language (EN / ZH)

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
        // If the executable name ends with "-en" or "_en" (e.g. tile-group-en.exe), default to English
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
        // Auto-detect via Windows UI Language (0x04 = LANG_CHINESE)
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

    /// Crop work area rectangle according to target region mode
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

/// Window group state manager / 窗口组状态管理器
struct GroupManager {
    windows: Vec<HWND>,
    original_rects: HashMap<HWND, RECT>,
    region: TileRegion,
    target_monitor: Option<HMONITOR>,
    lang: Lang,
}

impl GroupManager {
    fn new(lang: Lang) -> Self {
        Self {
            windows: Vec::new(),
            original_rects: HashMap::new(),
            region: TileRegion::FullWorkArea,
            target_monitor: None,
            lang,
        }
    }

    fn toggle_language(&mut self) {
        self.lang = self.lang.toggle();
        print_banner(self.lang);
        self.print_group_status();
    }

    /// Prune closed or invalid window handles
    fn prune_closed_windows(&mut self) -> bool {
        let before = self.windows.len();
        self.windows.retain(|&hwnd| unsafe { IsWindow(hwnd) != 0 });
        self.original_rects
            .retain(|&hwnd, _| unsafe { IsWindow(hwnd) != 0 });
        if self.windows.is_empty() {
            self.target_monitor = None;
        }
        self.windows.len() != before
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
            // Already in group -> remove and restore pre-grouped position
            self.windows.remove(pos);
            if let Some(orig) = self.original_rects.remove(&hwnd) {
                unsafe {
                    if IsZoomed(hwnd) != 0 || IsIconic(hwnd) != 0 {
                        restore_window(hwnd);
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
        } else {
            // Not in group -> save original rect and add to group
            let mut rect: RECT = unsafe { zeroed() };
            if unsafe { GetWindowRect(hwnd, &mut rect) } != 0 {
                self.original_rects.insert(hwnd, rect);
            }

            let mon = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
            if !mon.is_null() {
                self.target_monitor = Some(mon);
            }

            self.windows.push(hwnd);
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
        }
    }

    /// Minimize or restore the entire group at once
    fn toggle_minimize_group(&mut self) {
        self.prune_closed_windows();
        if self.windows.is_empty() {
            match self.lang {
                Lang::Zh => println!("ℹ️ 当前窗口组为空，请先按 Alt+G 将窗口加入组。"),
                Lang::En => println!("ℹ️ Window group is empty. Press Alt+G to add windows first."),
            }
            return;
        }

        let all_minimized = self
            .windows
            .iter()
            .all(|&hwnd| unsafe { IsIconic(hwnd) != 0 });

        if all_minimized {
            for &hwnd in &self.windows {
                unsafe {
                    restore_window(hwnd);
                }
            }
            self.retile();
            if let Some(&last_hwnd) = self.windows.last() {
                unsafe {
                    SetForegroundWindow(last_hwnd);
                }
            }
            match self.lang {
                Lang::Zh => println!("🔼 已一键还原整个窗口组 ({} 个窗口)", self.windows.len()),
                Lang::En => println!("🔼 Restored entire window group ({} window(s))", self.windows.len()),
            }
        } else {
            for &hwnd in &self.windows {
                unsafe {
                    ShowWindow(hwnd, SW_MINIMIZE);
                }
            }
            match self.lang {
                Lang::Zh => println!("🔽 已一键最小化整个窗口组 ({} 个窗口)", self.windows.len()),
                Lang::En => println!("🔽 Minimized entire window group ({} window(s))", self.windows.len()),
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
        self.target_monitor = None;
        match self.lang {
            Lang::Zh => println!("🧹 已清空窗口组，所有窗口恢复自由状态。"),
            Lang::En => println!("🧹 Cleared window group. All windows are now unmanaged."),
        }
    }

    /// Dynamically divide target region and tile all grouped windows
    fn retile(&mut self) {
        self.prune_closed_windows();
        let n = self.windows.len();
        if n == 0 {
            return;
        }

        let fg_hwnd = unsafe { GetForegroundWindow() };
        let work_area = self.get_target_work_area();
        let bounds = self.region.apply(work_area);
        let target_rects = calculate_grid_layout(bounds, n);

        for (&hwnd, target) in self.windows.iter().zip(target_rects.iter()) {
            unsafe {
                if IsIconic(hwnd) != 0 || IsZoomed(hwnd) != 0 {
                    restore_window(hwnd);
                }
                set_window_pos_compensated(hwnd, *target);
            }
        }

        if !fg_hwnd.is_null() && self.windows.contains(&fg_hwnd) {
            unsafe {
                SetForegroundWindow(fg_hwnd);
            }
        }
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

/// Reliably restore a minimized or maximized window (compatible with Windows Terminal ConPTY/XAML)
unsafe fn restore_window(hwnd: HWND) {
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
}

/// Calculate grid layout rectangles for `n` windows inside `bounds`:
/// - n = 1: Full region (1x1)
/// - n = 2: Side-by-side split (1x2)
/// - n = 3: Three equal columns (1x3)
/// - n = 4: 2x2 grid
/// - n >= 5: M rows x K columns even grid
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
        let cols_in_row = if r < extra { base_cols + 1 } else { base_cols };
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

/// Compensate for Windows 10/11 DWM invisible borders (~7px) for seamless zero-gap tiling
unsafe fn set_window_pos_compensated(hwnd: HWND, target: RECT) {
    SetWindowPos(
        hwnd,
        HWND_TOP,
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
            HWND_TOP,
            adj_left,
            adj_top,
            adj_width,
            adj_height,
            SWP_NOACTIVATE,
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

thread_local! {
    static MANAGER: RefCell<GroupManager> = RefCell::new(GroupManager::new(Lang::detect()));
}

/// Listen for system window destruction events to auto-retile remaining grouped windows
unsafe extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _id_event_thread: u32,
    _dwms_event_time: u32,
) {
    if event == EVENT_OBJECT_DESTROY && id_object == OBJID_WINDOW && id_child == 0 && !hwnd.is_null()
    {
        // Ensure the window is truly closed and not merely minimized (e.g. XAML/UIA teardown on minimize)
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
            }
        });
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

        if !(ok_g && ok_h && ok_s && ok_t && ok_c && ok_l && ok_q) {
            match lang {
                Lang::Zh => eprintln!("⚠️ 部分快捷键注册失败（可能与其他软件冲突）:"),
                Lang::En => eprintln!("⚠️ Failed to register some hotkeys (possible conflict):"),
            }
            eprintln!(
                "   Alt+G: {}, Alt+H: {}, Alt+S: {}, Alt+T: {}, Alt+C: {}, Alt+L: {}, Alt+Q: {}",
                ok_g, ok_h, ok_s, ok_t, ok_c, ok_l, ok_q
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
        ] {
            UnregisterHotKey(null_mut(), id);
        }
    }
}

fn print_banner(lang: Lang) {
    match lang {
        Lang::Zh => {
            println!("==============================================================");
            println!(" 🪟 TileGroup v0.1.2 - 轻量级按需窗口组平铺工具 (Rust)");
            println!("==============================================================");
            println!(" 快捷键说明：");
            println!("   [Alt + G]  将当前激活窗口 加入 / 移出 窗口组（自动均分平铺）");
            println!("   [Alt + H]  一键 隐藏(最小化) / 还原 整个窗口组");
            println!("   [Alt + S]  切换平铺区域（全屏 -> 左半屏 -> 右半屏 -> 上半屏 -> 下半屏）");
            println!("   [Alt + T]  手动重新对齐平铺当前组");
            println!("   [Alt + C]  清空窗口组（解散编组）");
            println!("   [Alt + L]  切换界面语言 (中文 / English)");
            println!("   [Alt + Q]  退出 TileGroup");
            println!("--------------------------------------------------------------");
            println!(" 💡 提示：组外窗口完全不受影响；关闭组内窗口会自动重排剩余窗口。");
            println!("==============================================================\n");
        }
        Lang::En => {
            println!("==============================================================");
            println!(" 🪟 TileGroup v0.1.2 - Opt-in Window Group Tiler for Windows");
            println!("==============================================================");
            println!(" Hotkeys:");
            println!("   [Alt + G]  Add / Remove active window to/from group & auto-tile");
            println!("   [Alt + H]  Hide (Minimize) / Restore the entire window group");
            println!("   [Alt + S]  Switch tiling region (Full -> Left -> Right -> Top -> Bottom)");
            println!("   [Alt + T]  Re-tile current group manually");
            println!("   [Alt + C]  Clear window group (ungroup all)");
            println!("   [Alt + L]  Switch UI language (English / 中文)");
            println!("   [Alt + Q]  Quit TileGroup");
            println!("--------------------------------------------------------------");
            println!(" 💡 Tip: Non-grouped windows are untouched; closing a grouped");
            println!("         window automatically re-tiles the remaining ones.");
            println!("==============================================================\n");
        }
    }
}

fn main() {
    // 1. Enable Per-Monitor V2 DPI Awareness
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    // 2. Disable Console QuickEdit Mode to prevent accidental terminal freeze on mouse click
    disable_console_quick_edit();

    let lang = MANAGER.with(|m| m.borrow().lang);
    print_banner(lang);
    register_hotkeys(lang);

    // Register window destroy hook to auto-retile when a grouped window is closed
    let hook = unsafe {
        SetWinEventHook(
            EVENT_OBJECT_DESTROY,
            EVENT_OBJECT_DESTROY,
            null_mut(),
            Some(win_event_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
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
                        MANAGER.with(|m| match m.borrow().lang {
                            Lang::Zh => println!("👋 正在退出 TileGroup..."),
                            Lang::En => println!("👋 Exiting TileGroup..."),
                        });
                        PostQuitMessage(0);
                    }
                    _ => {}
                }
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        if !hook.is_null() {
            UnhookWinEvent(hook);
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

        // N = 1: Full region
        let r1 = calculate_grid_layout(bounds, 1);
        assert_eq!(r1.len(), 1);
        assert_eq!((r1[0].left, r1[0].top, r1[0].right, r1[0].bottom), (0, 0, 1200, 800));

        // N = 2: 1x2 split
        let r2 = calculate_grid_layout(bounds, 2);
        assert_eq!(r2.len(), 2);
        assert_eq!((r2[0].left, r2[0].right), (0, 600));
        assert_eq!((r2[1].left, r2[1].right), (600, 1200));

        // N = 3: 1x3 columns
        let r3 = calculate_grid_layout(bounds, 3);
        assert_eq!(r3.len(), 3);
        assert_eq!((r3[0].left, r3[0].right), (0, 400));
        assert_eq!((r3[1].left, r3[1].right), (400, 800));
        assert_eq!((r3[2].left, r3[2].right), (800, 1200));

        // N = 4: 2x2 grid
        let r4 = calculate_grid_layout(bounds, 4);
        assert_eq!(r4.len(), 4);
        assert_eq!((r4[0].left, r4[0].top, r4[0].right, r4[0].bottom), (0, 0, 600, 400));
        assert_eq!((r4[1].left, r4[1].top, r4[1].right, r4[1].bottom), (600, 0, 1200, 400));
        assert_eq!((r4[2].left, r4[2].top, r4[2].right, r4[2].bottom), (0, 400, 600, 800));
        assert_eq!((r4[3].left, r4[3].top, r4[3].right, r4[3].bottom), (600, 400, 1200, 800));
    }
}
