# TileGroup 🪟

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

[English](README.md) | [简体中文](README_zh.md)

**A lightweight, non-intrusive, opt-in window group tiler for Windows.**

Unlike full-screen tiling window managers (such as GlazeWM or Komorebi) that hijack your entire desktop and force every window to tile unless blacklisted, **TileGroup** stays completely out of your way:

- **100% Non-Intrusive (Opt-in Only)**: Your Windows desktop behaves normally by default. Only the windows you explicitly add to the group are tiled—zero blacklist configuration needed.
- **Dynamic Group Tiling**: Add or remove any window to/from the group with a single hotkey. Windows in the group automatically divide the target region evenly:
  - `1` window: Full region (`1×1`)
  - `2` windows: Side-by-side split (`1×2`)
  - `3` windows: Three equal columns (`1×3`)
  - `4` windows: `2×2` grid
  - `5+` windows: Automatic `M×N` grid layout
- **Auto Re-tile on Window Close**: If you close a grouped window, the remaining windows in the group automatically re-tile to fill the space.
- **Auto Restore on Removal**: Removing a window from the group automatically restores it to its original pre-grouped size and position.
- **One-Key Group Minimize / Restore**: Minimize the entire window group at once, or bring the whole group back to the front together.
- **Pixel-Perfect & Ultra-Lightweight**: Built in pure Rust using `windows-sys` with Per-Monitor V2 DPI awareness and Windows 10/11 DWM invisible border compensation. Standalone `.exe` is only **~130 KB** with **~2 MB** RAM usage.

---

## ⌨️ Default Hotkeys

| Hotkey | Action |
| :--- | :--- |
| **`Alt + G`** | **Add / Remove** the active foreground window to/from the group and auto-tile (restores original window size on removal) |
| **`Alt + H`** | **Hide (Minimize) / Restore** the entire window group at once |
| **`Alt + S`** | **Switch Tiling Region** (`Full WorkArea` → `Left Half` → `Right Half` → `Top Half` → `Bottom Half`) and re-tile |
| **`Alt + T`** | **Re-tile** the current group manually (snap back if dragged out of place) |
| **`Alt + C`** | **Clear** the window group (ungroup all windows) |
| **`Alt + L`** | **Switch Language** between English and Chinese (`EN` / `ZH`) |
| **`Alt + Q`** | **Quit** TileGroup |

---

## 🚀 Usage & CLI Options

Run directly or pass `--en` / `--zh` to force the interface language (auto-detected from your Windows UI language by default):

```powershell
# Auto-detect language from Windows locale
tile-group.exe

# Force English UI
tile-group.exe --en

# Force Chinese UI
tile-group.exe --zh
```

---

## 🛠️ Building from Source

Requires Rust (`cargo`):

```powershell
git clone https://github.com/masuexe/tile-group.git
cd tile-group
cargo build --release
```

The compiled standalone binary will be located at `target/release/tile-group.exe`.

---

## 📄 License

[MIT License](LICENSE)
