//! Windows API layer: window enumeration, screen capture, background (non-intrusive)
//! input and UAC elevation.
//!
//! Everything is declared as raw FFI against user32/gdi32/kernel32/shell32 instead of
//! pulling in a wrapper crate: the surface we need is tiny, and it keeps the dependency
//! tree small (good for build times in CI and for the size of the resulting `.exe`).

#![allow(non_snake_case, clippy::upper_case_acronyms)]

use std::ptr::null_mut;

// ---------------------------------------------------------------- message constants
pub const WM_CLOSE: u32 = 0x0010;
pub const WM_MOUSEMOVE: u32 = 0x0200;
pub const WM_LBUTTONDOWN: u32 = 0x0201;
pub const WM_LBUTTONUP: u32 = 0x0202;
pub const WM_LBUTTONDBLCLK: u32 = 0x0203;
pub const WM_RBUTTONDOWN: u32 = 0x0204;
pub const WM_RBUTTONUP: u32 = 0x0205;
pub const WM_MBUTTONDOWN: u32 = 0x0207;
pub const WM_MBUTTONUP: u32 = 0x0208;
pub const WM_MOUSEWHEEL: u32 = 0x020A;
pub const WM_KEYDOWN: u32 = 0x0100;
pub const WM_KEYUP: u32 = 0x0101;
pub const WM_CHAR: u32 = 0x0102;
pub const WM_SYSKEYDOWN: u32 = 0x0104;
pub const WM_SYSKEYUP: u32 = 0x0105;

pub const MK_LBUTTON: usize = 0x0001;
pub const MK_RBUTTON: usize = 0x0002;
pub const MK_MBUTTON: usize = 0x0010;

pub const VK_BACK: u16 = 0x08;
pub const VK_TAB: u16 = 0x09;
pub const VK_RETURN: u16 = 0x0D;
pub const VK_SHIFT: u16 = 0x10;
pub const VK_CONTROL: u16 = 0x11;
pub const VK_MENU: u16 = 0x12;
pub const VK_PAUSE: u16 = 0x13;
pub const VK_CAPITAL: u16 = 0x14;
pub const VK_ESCAPE: u16 = 0x1B;
pub const VK_SPACE: u16 = 0x20;
pub const VK_PRIOR: u16 = 0x21;
pub const VK_NEXT: u16 = 0x22;
pub const VK_END: u16 = 0x23;
pub const VK_HOME: u16 = 0x24;
pub const VK_LEFT: u16 = 0x25;
pub const VK_UP: u16 = 0x26;
pub const VK_RIGHT: u16 = 0x27;
pub const VK_DOWN: u16 = 0x28;
pub const VK_SNAPSHOT: u16 = 0x2C;
pub const VK_INSERT: u16 = 0x2D;
pub const VK_DELETE: u16 = 0x2E;
pub const VK_LWIN: u16 = 0x5B;
pub const VK_RWIN: u16 = 0x5C;
pub const VK_F1: u16 = 0x70;

const SM_CXSCREEN: i32 = 0;
const SM_CYSCREEN: i32 = 1;
const SW_RESTORE: i32 = 9;
const GA_ROOT: u32 = 2;
const SRCCOPY: u32 = 0x00CC_0020;
const DIB_RGB_COLORS: u32 = 0;
const BI_RGB: u32 = 0;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const MAPVK_VK_TO_VSC: u32 = 0;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct POINT {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct RECT {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct BITMAPINFOHEADER {
    bi_size: u32,
    bi_width: i32,
    bi_height: i32,
    bi_planes: u16,
    bi_bit_count: u16,
    bi_compression: u32,
    bi_size_image: u32,
    bi_x_pels_per_meter: i32,
    bi_y_pels_per_meter: i32,
    bi_clr_used: u32,
    bi_clr_important: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct BITMAPINFO {
    bmi_header: BITMAPINFOHEADER,
    bmi_colors: [u32; 1],
}

#[link(name = "user32")]
extern "system" {
    fn EnumWindows(callback: Option<unsafe extern "system" fn(isize, isize) -> i32>, lparam: isize) -> i32;
    fn GetWindowTextW(hwnd: isize, buffer: *mut u16, max_count: i32) -> i32;
    fn IsWindowVisible(hwnd: isize) -> i32;
    fn IsWindow(hwnd: isize) -> i32;
    fn IsIconic(hwnd: isize) -> i32;
    fn GetWindowThreadProcessId(hwnd: isize, pid: *mut u32) -> u32;
    fn GetForegroundWindow() -> isize;
    fn GetAncestor(hwnd: isize, flags: u32) -> isize;
    fn ScreenToClient(hwnd: isize, point: *mut POINT) -> i32;
    fn GetClientRect(hwnd: isize, rect: *mut RECT) -> i32;
    fn WindowFromPoint(point: POINT) -> isize;
    fn PostMessageW(hwnd: isize, message: u32, wparam: usize, lparam: isize) -> i32;
    fn ShowWindow(hwnd: isize, cmd: i32) -> i32;
    fn SetForegroundWindow(hwnd: isize) -> i32;
    fn GetSystemMetrics(index: i32) -> i32;
    fn GetDC(hwnd: isize) -> isize;
    fn ReleaseDC(hwnd: isize, hdc: isize) -> i32;
    fn VkKeyScanW(character: u16) -> i16;
    fn MapVirtualKeyW(code: u32, map_type: u32) -> u32;
    fn GetKeyState(vk: i32) -> i16;
    fn MessageBeep(kind: u32) -> i32;
}

#[link(name = "gdi32")]
extern "system" {
    fn CreateCompatibleDC(hdc: isize) -> isize;
    fn CreateCompatibleBitmap(hdc: isize, width: i32, height: i32) -> isize;
    fn SelectObject(hdc: isize, object: isize) -> isize;
    fn BitBlt(dest: isize, x: i32, y: i32, width: i32, height: i32, src: isize, src_x: i32, src_y: i32, rop: u32) -> i32;
    fn GetDIBits(hdc: isize, bitmap: isize, start: u32, lines: u32, bits: *mut u8, info: *mut BITMAPINFO, usage: u32) -> i32;
    fn DeleteObject(object: isize) -> i32;
    fn DeleteDC(hdc: isize) -> i32;
}

#[link(name = "dwmapi")]
extern "system" {
    fn DwmSetWindowAttribute(hwnd: isize, attribute: u32, value: *const std::ffi::c_void, size: u32) -> i32;
}

const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
const DWMWCP_ROUND: i32 = 2;

#[link(name = "kernel32")]
extern "system" {
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
    fn QueryFullProcessImageNameW(process: isize, flags: u32, name: *mut u16, size: *mut u32) -> i32;
    fn CloseHandle(handle: isize) -> i32;
}

#[link(name = "shell32")]
extern "system" {
    fn IsUserAnAdmin() -> i32;
}

#[repr(C)]
struct OsVersionInfoW {
    size: u32,
    major: u32,
    minor: u32,
    build: u32,
    platform: u32,
    service_pack: [u16; 128],
}

#[link(name = "ntdll")]
extern "system" {
    fn RtlGetVersion(info: *mut OsVersionInfoW) -> i32;
}

/// Real OS build number (`RtlGetVersion` ignores the compatibility shim that makes
/// `GetVersionEx` lie on Windows 10+). 22000 and above means Windows 11.
pub fn windows_build() -> u32 {
    let mut info = OsVersionInfoW {
        size: std::mem::size_of::<OsVersionInfoW>() as u32,
        major: 0,
        minor: 0,
        build: 0,
        platform: 0,
        service_pack: [0; 128],
    };
    let status = unsafe { RtlGetVersion(&mut info) };
    if status == 0 {
        info.build
    } else {
        0
    }
}

#[link(name = "bcrypt")]
extern "system" {
    fn BCryptGenRandom(algorithm: isize, buffer: *mut u8, len: u32, flags: u32) -> i32;
}

// ---------------------------------------------------------------- helpers

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Random hex string of `bytes * 2` characters, sourced from the OS CSPRNG.
pub fn random_hex(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 0x0000_0002;
    let status = unsafe { BCryptGenRandom(0, buffer.as_mut_ptr(), buffer.len() as u32, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
    if status != 0 {
        // Extremely unlikely; fall back to a time-derived seed rather than failing to start.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        for (index, byte) in buffer.iter_mut().enumerate() {
            *byte = ((nanos >> ((index % 16) * 8)) & 0xFF) as u8 ^ (index as u8 * 37);
        }
    }
    let mut out = String::with_capacity(bytes * 2);
    for byte in buffer {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

// ---------------------------------------------------------------- windows

#[derive(Clone, serde::Serialize, serde::Deserialize, Debug)]
pub struct WindowInfo {
    pub hwnd: i64,
    pub title: String,
    pub pid: u32,
    pub process: String,
}

fn window_title(hwnd: isize) -> String {
    let mut buffer = [0u16; 512];
    let length = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    if length <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buffer[..length as usize])
}

fn process_name(pid: u32) -> String {
    if pid == 0 {
        return String::new();
    }
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle == 0 {
            return String::new();
        }
        let mut buffer = [0u16; 1024];
        let mut size = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut size);
        CloseHandle(handle);
        if ok == 0 {
            return String::new();
        }
        let full = String::from_utf16_lossy(&buffer[..size as usize]);
        full.rsplit('\\').next().unwrap_or_default().to_string()
    }
}

fn describe(hwnd: isize) -> Option<WindowInfo> {
    let title = window_title(hwnd);
    if title.is_empty() {
        return None;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    Some(WindowInfo { hwnd: hwnd as i64, title, pid, process: process_name(pid) })
}

unsafe extern "system" fn enum_callback(hwnd: isize, lparam: isize) -> i32 {
    if IsWindowVisible(hwnd) == 0 {
        return 1;
    }
    if let Some(info) = describe(hwnd) {
        let sink = &mut *(lparam as *mut Vec<WindowInfo>);
        sink.push(info);
    }
    1
}

/// Visible top-level windows with a non-empty title, capped at 120 entries.
pub fn list_windows() -> Vec<WindowInfo> {
    let mut sink: Vec<WindowInfo> = Vec::new();
    unsafe {
        EnumWindows(Some(enum_callback), &mut sink as *mut Vec<WindowInfo> as isize);
    }
    sink.truncate(120);
    sink
}

pub fn active_window() -> WindowInfo {
    let hwnd = unsafe { GetForegroundWindow() };
    describe(hwnd).unwrap_or(WindowInfo { hwnd: hwnd as i64, title: String::new(), pid: 0, process: String::new() })
}

pub fn close_window(hwnd: isize) -> bool {
    if hwnd == 0 || unsafe { IsWindow(hwnd) } == 0 {
        return false;
    }
    unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) != 0 }
}

pub fn activate_window(hwnd: isize) -> bool {
    if hwnd == 0 || unsafe { IsWindow(hwnd) } == 0 {
        return false;
    }
    unsafe {
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        SetForegroundWindow(hwnd) != 0
    }
}

// ---------------------------------------------------------------- screen capture

pub struct Shot {
    pub jpeg: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Captures the primary screen with GDI and encodes it as JPEG.
pub fn capture(max_width: u32) -> Result<Shot, String> {
    unsafe {
        let width = GetSystemMetrics(SM_CXSCREEN);
        let height = GetSystemMetrics(SM_CYSCREEN);
        if width <= 0 || height <= 0 {
            return Err("не удалось определить размер экрана".into());
        }
        let screen = GetDC(0);
        if screen == 0 {
            return Err("GetDC вернул пустой контекст устройства".into());
        }
        let memory = CreateCompatibleDC(screen);
        let bitmap = CreateCompatibleBitmap(screen, width, height);
        if memory == 0 || bitmap == 0 {
            ReleaseDC(0, screen);
            return Err("не удалось создать совместимый контекст GDI".into());
        }
        let previous = SelectObject(memory, bitmap);
        let copied = BitBlt(memory, 0, 0, width, height, screen, 0, 0, SRCCOPY);
        if copied == 0 {
            SelectObject(memory, previous);
            DeleteObject(bitmap);
            DeleteDC(memory);
            ReleaseDC(0, screen);
            return Err("BitBlt не смог скопировать экран".into());
        }

        let mut info = BITMAPINFO {
            bmi_header: BITMAPINFOHEADER {
                bi_size: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                bi_width: width,
                bi_height: -height, // negative height => top-down rows
                bi_planes: 1,
                bi_bit_count: 32,
                bi_compression: BI_RGB,
                bi_size_image: 0,
                bi_x_pels_per_meter: 0,
                bi_y_pels_per_meter: 0,
                bi_clr_used: 0,
                bi_clr_important: 0,
            },
            bmi_colors: [0],
        };

        let mut bgra = vec![0u8; (width as usize) * (height as usize) * 4];
        let lines = GetDIBits(
            memory,
            bitmap,
            0,
            height as u32,
            bgra.as_mut_ptr(),
            &mut info,
            DIB_RGB_COLORS,
        );

        SelectObject(memory, previous);
        DeleteObject(bitmap);
        DeleteDC(memory);
        ReleaseDC(0, screen);

        if lines == 0 {
            return Err("GetDIBits не вернул пиксели".into());
        }

        let mut rgb = Vec::with_capacity((width as usize) * (height as usize) * 3);
        for pixel in bgra.chunks_exact(4) {
            rgb.push(pixel[2]);
            rgb.push(pixel[1]);
            rgb.push(pixel[0]);
        }

        let image = image::RgbImage::from_raw(width as u32, height as u32, rgb)
            .ok_or_else(|| "не удалось собрать изображение из пикселей".to_string())?;
        let image = if image.width() > max_width.max(160) {
            let ratio = max_width.max(160) as f64 / image.width() as f64;
            let target_height = ((image.height() as f64 * ratio).round() as u32).max(1);
            image::imageops::resize(&image, max_width.max(160), target_height, image::imageops::FilterType::Triangle)
        } else {
            image
        };

        let mut out = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut out);
        image
            .write_to(&mut cursor, image::ImageFormat::Jpeg)
            .map_err(|error| format!("не удалось закодировать JPEG: {error}"))?;

        Ok(Shot { jpeg: out, width: width as u32, height: height as u32 })
    }
}

// ---------------------------------------------------------------- background input

#[derive(Debug)]
pub struct InputError(pub String);

impl From<InputError> for String {
    fn from(error: InputError) -> String {
        error.0
    }
}

impl std::fmt::Display for InputError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn pack_point(x: i32, y: i32) -> isize {
    (((y & 0xFFFF) << 16) | (x & 0xFFFF)) as isize
}

pub fn hwnd_at(x: i32, y: i32) -> isize {
    unsafe {
        let hwnd = WindowFromPoint(POINT { x, y });
        if hwnd == 0 || IsWindow(hwnd) == 0 || IsWindowVisible(hwnd) == 0 {
            return 0;
        }
        hwnd
    }
}

pub fn post(hwnd: isize, message: u32, wparam: usize, lparam: isize) -> bool {
    if hwnd == 0 {
        return false;
    }
    unsafe { PostMessageW(hwnd, message, wparam, lparam) != 0 }
}

fn client_lparam(hwnd: isize, x: i32, y: i32, require_inside: bool) -> Result<isize, InputError> {
    let mut point = POINT { x, y };
    if unsafe { ScreenToClient(hwnd, &mut point) } == 0 {
        return Err(InputError("не удалось преобразовать координаты окна".into()));
    }
    if require_inside {
        let mut rect = RECT::default();
        if unsafe { GetClientRect(hwnd, &mut rect) } == 0 {
            return Err(InputError("не удалось получить клиентскую область окна".into()));
        }
        let inside = point.x >= 0 && point.y >= 0 && point.x < rect.right && point.y < rect.bottom;
        if !inside {
            return Err(InputError(
                "эта точка находится вне клиентской области окна; заголовки/рамки не поддерживаются".into(),
            ));
        }
    }
    Ok(pack_point(point.x, point.y))
}

pub fn post_mouse(hwnd: isize, message: u32, wparam: usize, x: i32, y: i32, require_inside: bool) -> Result<(), InputError> {
    if unsafe { IsWindow(hwnd) } == 0 {
        return Err(InputError("целевое окно закрылось или недоступно".into()));
    }
    let lparam = client_lparam(hwnd, x, y, require_inside)?;
    if !post(hwnd, message, wparam, lparam) {
        return Err(InputError(format!("Windows отклонила фоновое сообщение для окна {hwnd:#x}")));
    }
    Ok(())
}

pub fn mouse_spec(button: &str) -> Option<(u32, u32, usize)> {
    match button {
        "left" | "primary" => Some((WM_LBUTTONDOWN, WM_LBUTTONUP, MK_LBUTTON)),
        "right" | "secondary" => Some((WM_RBUTTONDOWN, WM_RBUTTONUP, MK_RBUTTON)),
        "middle" => Some((WM_MBUTTONDOWN, WM_MBUTTONUP, MK_MBUTTON)),
        _ => None,
    }
}

pub fn mouse_down(hwnd: isize, spec: (u32, u32, usize), x: i32, y: i32) -> Result<(), InputError> {
    let lparam = client_lparam(hwnd, x, y, true)?;
    if !post(hwnd, spec.0, spec.2, lparam) {
        return Err(InputError("фоновое нажатие не отправлено".into()));
    }
    Ok(())
}

pub fn mouse_up(hwnd: isize, spec: (u32, u32, usize), x: i32, y: i32) {
    if let Ok(lparam) = client_lparam(hwnd, x, y, false) {
        post(hwnd, spec.1, 0, lparam);
    }
}

pub fn double_click_marker(hwnd: isize, spec: (u32, u32, usize), x: i32, y: i32) {
    if let Ok(lparam) = client_lparam(hwnd, x, y, true) {
        post(hwnd, WM_LBUTTONDBLCLK, spec.2, lparam);
    }
}

pub fn key_codes(key: &str) -> Option<(u16, Vec<u16>)> {
    let name = key.trim().to_lowercase();
    let alias = match name.as_str() {
        "enter" | "return" => Some(VK_RETURN),
        "tab" => Some(VK_TAB),
        "space" => Some(VK_SPACE),
        "backspace" => Some(VK_BACK),
        "delete" | "del" => Some(VK_DELETE),
        "esc" | "escape" => Some(VK_ESCAPE),
        "shift" => Some(VK_SHIFT),
        "ctrl" | "control" => Some(VK_CONTROL),
        "alt" => Some(VK_MENU),
        "win" | "super" | "meta" => Some(VK_LWIN),
        "left" => Some(VK_LEFT),
        "right" => Some(VK_RIGHT),
        "up" => Some(VK_UP),
        "down" => Some(VK_DOWN),
        "home" => Some(VK_HOME),
        "end" => Some(VK_END),
        "pageup" => Some(VK_PRIOR),
        "pagedown" => Some(VK_NEXT),
        "insert" => Some(VK_INSERT),
        "capslock" => Some(VK_CAPITAL),
        "pause" => Some(VK_PAUSE),
        "printscreen" => Some(VK_SNAPSHOT),
        _ => None,
    };
    if let Some(code) = alias {
        return Some((code, Vec::new()));
    }
    if name.len() >= 2 && name.starts_with('f') {
        if let Ok(number) = name[1..].parse::<u16>() {
            if (1..=24).contains(&number) {
                return Some((VK_F1 + number - 1, Vec::new()));
            }
        }
    }
    let chars: Vec<char> = key.chars().collect();
    if chars.len() == 1 {
        let unit = chars[0] as u32;
        if unit <= 0xFFFF {
            let scan = unsafe { VkKeyScanW(unit as u16) };
            if scan != -1 {
                let vk = (scan as u16) & 0xFF;
                let state = ((scan as u16) >> 8) & 0xFF;
                let mut modifiers = Vec::new();
                if state & 1 != 0 {
                    modifiers.push(VK_SHIFT);
                }
                if state & 2 != 0 {
                    modifiers.push(VK_CONTROL);
                }
                if state & 4 != 0 {
                    modifiers.push(VK_MENU);
                }
                return Some((vk, modifiers));
            }
        }
    }
    None
}

pub fn post_key(hwnd: isize, vk: u16, down: bool) -> bool {
    let scan = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) } as isize;
    let mut lparam = 1isize | (scan << 16);
    if !down {
        lparam |= 0xC000_0000u32 as isize;
    }
    let message = if vk == VK_MENU {
        if down {
            WM_SYSKEYDOWN
        } else {
            WM_SYSKEYUP
        }
    } else if down {
        WM_KEYDOWN
    } else {
        WM_KEYUP
    };
    post(hwnd, message, vk as usize, lparam)
}

pub fn post_char(hwnd: isize, unit: u16) -> bool {
    post(hwnd, WM_CHAR, unit as usize, 1)
}

pub fn post_wheel_root(hwnd: isize, x: i32, y: i32, clicks: i32) -> Result<(), InputError> {
    let root = unsafe {
        let ancestor = GetAncestor(hwnd, GA_ROOT);
        if ancestor != 0 {
            ancestor
        } else {
            hwnd
        }
    };
    let delta = (clicks * 120).clamp(-1200, 1200);
    let wparam = (((delta & 0xFFFF) << 16) as i32) as usize;
    let lparam = pack_point(x, y);
    if !post(root, WM_MOUSEWHEEL, wparam, lparam) {
        return Err(InputError("фоновая прокрутка не выполнена".into()));
    }
    Ok(())
}

pub fn shifted() -> bool {
    unsafe { GetKeyState(VK_SHIFT as i32) & 0x8000u16 as i16 != 0 }
}

pub fn beep() {
    unsafe {
        MessageBeep(0);
    }
}

/// Screen size, used by the virtual cursor overlay to clamp coordinates.
pub fn screen_size() -> (i32, i32) {
    unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) }
}

// ---------------------------------------------------------------- elevation

/// The executable carries `requireAdministrator` in its manifest, so this is
/// normally true; it is shown in the UI as a fact, not as something to toggle.
pub fn is_elevated() -> bool {
    unsafe { IsUserAnAdmin() != 0 }
}

// ---------------------------------------------------------------- window shape

/// Asks DWM for native (anti-aliased) rounded corners, the Windows 11 way.
pub fn prefer_round_corners(hwnd: isize) {
    let preference = DWMWCP_ROUND;
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &preference as *const i32 as *const std::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        );
    }
}
