//! Optional PROJ backend, loaded dynamically at startup.
//!
//! This module is intentionally dependency-free: on Windows it resolves the
//! PROJ C API through `LoadLibraryExW` / `GetProcAddress` (kernel32); on Unix
//! it uses `dlopen` / `dlsym` (libc). The crate builds offline with no
//! `libloading` / build-script and PROJ stays an optional runtime component
//! (EPSG:4326 <-> EPSG:3857 keep using the dependency-free [`crate::projection::mercator`]
//! path when PROJ is absent).
//!
//! The public surface (`init_proj`, `proj`, `Proj`, `PjTransform`) is consumed
//! by [`crate::projection::Transformer::new_any`] to reproject arbitrary CRS pairs such as
//! CGCS2000 Gauss-Krüger zones (EPSG:45xx) against lon/lat.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

// ---- Platform-specific library loading ----------------------------------------

#[cfg(windows)]
mod platform {
    use std::ffi::{c_void, CString};
    use std::os::windows::ffi::OsStrExt;

    #[allow(non_camel_case_types)]
    mod windows_targets {
        use std::ffi::c_void;
        pub type HMODULE = *mut c_void;
        #[link(name = "kernel32")]
        extern "system" {
            pub fn LoadLibraryExW(
                lpLibFileName: *const u16,
                hFile: *mut c_void,
                dwFlags: u32,
            ) -> HMODULE;
            pub fn GetProcAddress(hModule: HMODULE, lpProcName: *const i8) -> *mut c_void;
        }
        pub const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 0x0000_0008;
    }

    pub type LibHandle = *mut c_void;

    pub fn load_library(path: &std::path::Path) -> Option<LibHandle> {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let hmodule = unsafe {
            windows_targets::LoadLibraryExW(
                wide.as_ptr(),
                std::ptr::null_mut(),
                windows_targets::LOAD_WITH_ALTERED_SEARCH_PATH,
            )
        };
        if hmodule.is_null() {
            None
        } else {
            Some(hmodule as LibHandle)
        }
    }

    pub unsafe fn get_symbol(handle: LibHandle, name: &str) -> Option<*mut c_void> {
        let c_name = CString::new(name).ok()?;
        let ptr = windows_targets::GetProcAddress(handle as _, c_name.as_ptr());
        if ptr.is_null() {
            None
        } else {
            Some(ptr as *mut c_void)
        }
    }
}

#[cfg(unix)]
mod platform {
    use std::ffi::{c_void, CString, CStr};

    pub type LibHandle = *mut c_void;

    pub fn load_library(path: &std::path::Path) -> Option<LibHandle> {
        let c_path = CString::new(path.to_string_lossy().as_bytes()).ok()?;
        let handle = unsafe { libc::dlopen(c_path.as_ptr(), libc::RTLD_LAZY) };
        if handle.is_null() {
            let err = unsafe { libc::dlerror() };
            if !err.is_null() {
                let msg = unsafe { CStr::from_ptr(err) }.to_string_lossy();
                eprintln!("[projection] dlopen failed: {}", msg);
            }
            None
        } else {
            Some(handle as LibHandle)
        }
    }

    pub unsafe fn get_symbol(handle: LibHandle, name: &str) -> Option<*mut c_void> {
        let c_name = CString::new(name).ok()?;
        let ptr = libc::dlsym(handle as _, c_name.as_ptr());
        if ptr.is_null() {
            None
        } else {
            Some(ptr as *mut c_void)
        }
    }
}

use platform::LibHandle;

// ---- PROJ C ABI we actually use -------------------------------------------------
// struct PJ_COORD is a union whose largest member (PJ_UVWXYZT) is 7 doubles.
// We model it as that full 56-byte layout so by-value FFI matches the callee;
// the leading two doubles are the (x, y) / (lon, lat) pair for 2D transforms.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct PJCoord {
    x: f64,
    y: f64,
    z: f64,
    t: f64,
    _pad: [f64; 3],
}

// proj_trans direction: PJ_FWD == 1 (see proj.h `enum pj_directions`).
const PJ_FWD: c_int = 1;
// proj_as_proj_string format: PJ_TEXT_PROJ == 0 (see `enum pj_proj_text_format`),
// which yields the classic `+proj=... ` string that proj4.js can consume directly.
const PJ_TEXT_PROJ: c_int = 0;

type FnCtxCreate = unsafe extern "C" fn() -> *mut c_void;
type FnSetSearchPaths =
    unsafe extern "C" fn(*mut c_void, c_int, *const *const c_char) -> c_int;
type FnCreateCrsToCrs =
    unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char, *mut c_void) -> *mut c_void;
type FnNormalize = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
type FnTrans = unsafe extern "C" fn(*mut c_void, c_int, PJCoord) -> PJCoord;
type FnDestroy = unsafe extern "C" fn(*mut c_void);
// Optional CRS-introspection entry points (present in every PROJ 6+ build,
// including proj_9.dll). Resolved non-fatally: if a symbol is missing the
// backend still loads and only [`Proj::crs_proj4`] returns `None`.
type FnProjCreate = unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void;
type FnAsProjString =
    unsafe extern "C" fn(*mut c_void, *mut c_void, c_int, *const *const c_char) -> *mut c_char;
type FnStringFree = unsafe extern "C" fn(*mut c_void);

/// A loaded PROJ library plus a shared context and the resolved entry points.
pub struct Proj {
    #[allow(dead_code)]
    lib: LibHandle,
    ctx: *mut c_void,
    create_lock: Mutex<()>,
    f_create: FnCreateCrsToCrs,
    f_normalize: FnNormalize,
    f_trans: FnTrans,
    f_destroy: FnDestroy,
    // CRS -> definition export (optional; None disables [`crs_proj4`] only).
    f_proj_create: Option<FnProjCreate>,
    f_as_proj_string: Option<FnAsProjString>,
    f_string_free: Option<FnStringFree>,
}

// The context is only used for object creation, serialized by `create_lock`;
// each [`PjTransform`] owns a distinct `PJ*` and is used single-threaded.
unsafe impl Send for Proj {}
unsafe impl Sync for Proj {}

/// A ready-to-use coordinate transform between two CRSs (visualization order:
/// geographic = lon/lat, projected = easting/northing).
pub struct PjTransform {
    pj: *mut c_void,
    proj: Arc<Proj>,
}

// Each transform is used by exactly one thread at a time (created and consumed
// within a single request), so it is safe to move between threads.
unsafe impl Send for PjTransform {}

impl Drop for PjTransform {
    fn drop(&mut self) {
        if !self.pj.is_null() {
            unsafe { (self.proj.f_destroy)(self.pj) };
        }
    }
}

impl Proj {
    /// Build a forward transform `from -> to` (both PROJ authority strings or
    /// WKT). Returns `None` if PROJ cannot construct the pipeline.
    pub fn crs_to_crs(this: &Arc<Proj>, from: &str, to: &str) -> Option<PjTransform> {
        let src = CString::new(from).ok()?;
        let dst = CString::new(to).ok()?;
        // Serialize context use: PROJ contexts are not safe for concurrent
        // object creation across threads.
        let _g = this.create_lock.lock().ok()?;
        let raw = unsafe { (this.f_create)(this.ctx, src.as_ptr(), dst.as_ptr(), std::ptr::null_mut()) };
        if raw.is_null() {
            return None;
        }
        // Force visualization order (lon/lat, x/east first) regardless of the
        // CRS's native axis order; this returns a new PJ and we drop the raw.
        let normalized = unsafe { (this.f_normalize)(this.ctx, raw) };
        unsafe { (this.f_destroy)(raw) };
        if normalized.is_null() {
            return None;
        }
        Some(PjTransform {
            pj: normalized,
            proj: Arc::clone(this),
        })
    }

    /// Export a single CRS's definition as a proj4-style string (PROJ
    /// `PJ_TEXT_PROJ`), e.g. `EPSG:4522` ->
    /// `+proj=tmerc +lat_0=0 +lon_0=102 +k=1 +x_0=34500000 +y_0=0 ...`.
    /// Used to let tile clients register an arbitrary projected CRS from the
    /// grid definition instead of hard-coding it. Returns `None` when the
    /// introspection symbols are unavailable or PROJ cannot build the object
    /// (unknown code, no proj.db), so callers degrade gracefully.
    pub fn crs_proj4(&self, crs: &str) -> Option<String> {
        // proj_create + proj_as_proj_string are required; proj_string_free is
        // optional (some PROJ builds omit it) — when absent the exported string
        // simply isn't reclaimed here. This path is called rarely and its result
        // is cached by the HTTP layer, so the leak is negligible.
        let (Some(f_create), Some(f_str)) = (self.f_proj_create, self.f_as_proj_string) else {
            eprintln!("[projection] crs_proj4({crs}): export symbols missing (create={:?} str={:?})", self.f_proj_create.map(|_| 1), self.f_as_proj_string.map(|_| 1));
            return None;
        };
        let f_free = self.f_string_free;
        let c = CString::new(crs).ok()?;
        // Serialize context use, as in [`Proj::crs_to_crs`].
        let _g = self.create_lock.lock().ok()?;
        let p = unsafe { f_create(self.ctx, c.as_ptr()) };
        if p.is_null() {
            eprintln!("[projection] crs_proj4({crs}): proj_create returned NULL");
            return None;
        }
        // options = NULL (single `const char **`), format = PJ_TEXT_PROJ.
        let raw = unsafe { f_str(self.ctx, p, PJ_TEXT_PROJ, std::ptr::null()) };
        let out = if raw.is_null() {
            None
        } else {
            Some(unsafe { CStr::from_ptr(raw) }.to_string_lossy().into_owned())
        };
        if let Some(f_free) = f_free {
            unsafe { f_free(raw as *mut c_void) };
        }
        unsafe { (self.f_destroy)(p) };
        out.filter(|s| !s.trim().is_empty())
    }
}

impl PjTransform {
    /// Forward-transform one `(x, y)` (input order matches the request CRS as
    /// normalized: geographic -> lon/lat, projected -> east/north).
    pub fn transform(&self, x: f64, y: f64) -> (f64, f64) {
        let c = PJCoord {
            x,
            y,
            z: 0.0,
            t: 0.0,
            _pad: [0.0; 3],
        };
        let r = unsafe { (self.proj.f_trans)(self.pj, PJ_FWD, c) };
        (r.x, r.y)
    }
}

/// Batch helper mirroring [`PjTransform::transform`].
impl PjTransform {
    pub fn transform_batch(&self, pts: &[(f64, f64)]) -> Vec<(f64, f64)> {
        pts.iter().map(|&(x, y)| self.transform(x, y)).collect()
    }
}

static PROJ: OnceLock<Option<Arc<Proj>>> = OnceLock::new();

/// Load the PROJ library (or the given path) and initialize the shared backend.
/// Safe to call once at startup; missing/failed load leaves the built-in
/// 4326<->3857 path working and simply disables arbitrary-CRS support.
pub fn init_proj(lib_path: Option<&str>, data_dir: Option<&str>) {
    let _ = PROJ.set(try_load(lib_path, data_dir));
}

/// The loaded backend, if any.
pub fn proj() -> Option<&'static Arc<Proj>> {
    PROJ.get()?.as_ref()
}

/// Export a CRS definition as a proj4 string via the loaded backend.
/// `None` when PROJ is not loaded, the symbols are missing, or the code is
/// unknown — callers then fall back to any built-in definition they hold.
pub fn crs_proj4(crs: &str) -> Option<String> {
    proj()?.crs_proj4(crs)
}

/// PROJ 动态库候选路径，按优先级排列：
/// 显式参数 > `PROJ_DLL` > 裸库名（工作目录 / 系统搜索路径，保留旧行为）> 可执行文件目录。
/// 最后一项对应安装包布局 `bin/map-api.exe` + `bin/proj_9.dll`，使服务从任意 CWD
/// 启动都能找到 PROJ。
fn lib_candidates(
    explicit: Option<&str>,
    env_dll: Option<&str>,
    exe_dir: Option<&Path>,
    default_name: &str,
) -> Vec<PathBuf> {
    if let Some(p) = explicit {
        return vec![PathBuf::from(p)];
    }
    let mut out = Vec::new();
    if let Some(p) = env_dll.filter(|s| !s.is_empty()) {
        out.push(PathBuf::from(p));
    }
    out.push(PathBuf::from(default_name));
    if let Some(dir) = exe_dir {
        out.push(dir.join(default_name));
    }
    out
}

/// PROJ 数据目录（含 `proj.db`）候选路径，按优先级排列：
/// 显式参数 > `PROJ_DATA` > `<cwd>/share/proj`（旧行为）> `<exe>/share/proj` >
/// `<exe>/../share/proj`（安装包布局：`bin/` 与 `share/proj/` 同级）。
/// 注意不读 `PROJ_LIB`：该变量常被系统 PostGIS 的旧版 proj.db 占用。
fn data_candidates(
    explicit: Option<&str>,
    env_data: Option<&str>,
    cwd: Option<&Path>,
    exe_dir: Option<&Path>,
) -> Vec<PathBuf> {
    if let Some(p) = explicit {
        return vec![PathBuf::from(p)];
    }
    let mut out = Vec::new();
    if let Some(p) = env_data.filter(|s| !s.is_empty()) {
        out.push(PathBuf::from(p));
    }
    if let Some(dir) = cwd {
        out.push(dir.join("share").join("proj"));
    }
    if let Some(dir) = exe_dir {
        out.push(dir.join("share").join("proj"));
        out.push(dir.join("..").join("share").join("proj"));
    }
    out
}

fn try_load(lib_path: Option<&str>, data_dir: Option<&str>) -> Option<Arc<Proj>> {
    // 显式参数 > 环境变量（PROJ_DLL / PROJ_DATA）> 运行目录默认值 > 可执行文件旁。
    #[cfg(windows)]
    let default_name = "proj_9.dll";
    #[cfg(unix)]
    let default_name = "libproj.so";

    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));
    let env_dll = std::env::var("PROJ_DLL").ok();
    let env_data = std::env::var("PROJ_DATA").ok();
    let cwd = std::env::current_dir().ok();

    // 逐个候选尝试加载：裸名字保留“工作目录 / 系统搜索路径”的既有行为，失败后
    // 再试可执行文件所在目录（安装包布局 bin/proj_9.dll，服务从任意 CWD 启动都能命中）。
    let lib = lib_candidates(lib_path, env_dll.as_deref(), exe_dir.as_deref(), default_name)
        .into_iter()
        .find_map(|c| {
            let h = platform::load_library(&c);
            if h.is_none() {
                eprintln!("[projection] PROJ load failed: cannot load '{}'", c.display());
            }
            h
        })?;
    if lib.is_null() {
        eprintln!("[projection] PROJ load failed: load_library returned NULL");
        return None;
    }
    // 显式参数 / 环境变量按原样使用（与旧行为一致）；未指定时按运行目录与安装
    // 布局查找首个**存在**的 proj.db 目录。
    let data_explicit = data_dir.or(env_data.as_deref()).is_some();
    let data_candidates = data_candidates(data_dir, env_data.as_deref(), cwd.as_deref(), exe_dir.as_deref());
    let data_dir = if data_explicit {
        data_candidates.into_iter().next()
    } else {
        data_candidates.into_iter().find(|d| d.is_dir())
    };
    // Safety: resolve the entry points we need; cast raw pointers to fn ptrs.
    macro_rules! sym {
        ($name:literal, $ty:ty) => {{
            let p = unsafe { platform::get_symbol(lib, $name) };
            if p.is_none() {
                eprintln!("[projection] PROJ load failed: symbol '{}' missing", $name);
                return None;
            }
            unsafe { std::mem::transmute::<*mut c_void, $ty>(p.unwrap()) }
        }};
    }
    let f_ctx_create: FnCtxCreate = sym!("proj_context_create", FnCtxCreate);
    let f_set_paths: FnSetSearchPaths = sym!("proj_context_set_search_paths", FnSetSearchPaths);
    let f_create: FnCreateCrsToCrs = sym!("proj_create_crs_to_crs", FnCreateCrsToCrs);
    let f_normalize: FnNormalize = sym!("proj_normalize_for_visualization", FnNormalize);
    let f_trans: FnTrans = sym!("proj_trans", FnTrans);
    let f_destroy: FnDestroy = sym!("proj_destroy", FnDestroy);
    // Optional (non-fatal) CRS-definition export symbols.
    macro_rules! sym_opt {
        ($name:literal, $ty:ty) => {{
            let p = unsafe { platform::get_symbol(lib, $name) };
            if p.is_none() {
                None
            } else {
                Some(unsafe { std::mem::transmute::<*mut c_void, $ty>(p.unwrap()) })
            }
        }};
    }
    let f_proj_create: Option<FnProjCreate> = sym_opt!("proj_create", FnProjCreate);
    let f_as_proj_string: Option<FnAsProjString> = sym_opt!("proj_as_proj_string", FnAsProjString);
    let f_string_free: Option<FnStringFree> = sym_opt!("proj_string_free", FnStringFree);

    let ctx = unsafe { f_ctx_create() };
    if ctx.is_null() {
        eprintln!("[projection] PROJ load failed: proj_context_create returned NULL");
        return None;
    }
    if let Some(dir) = data_dir.as_deref() {
        if let Ok(cs) = CString::new(dir.to_string_lossy().as_bytes()) {
            let arr = [cs.as_ptr() as *const c_char];
            unsafe { f_set_paths(ctx, 1, arr.as_ptr()) };
        }
    }
    Some(Arc::new(Proj {
        lib,
        ctx,
        create_lock: Mutex::new(()),
        f_create,
        f_normalize,
        f_trans,
        f_destroy,
        f_proj_create,
        f_as_proj_string,
        f_string_free,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    // A real, on-disk PROJ is not guaranteed in unit tests; this only checks
    // the built-in-free API compiles and that a missing backend degrades
    // gracefully (returns None rather than panicking).
    #[test]
    fn init_is_idempotent_and_nonfatal() {
        // bogus path -> load fails -> proj() stays None, no panic.
        init_proj(Some("definitely_not_proj_lib"), None);
        // second init is ignored (OnceLock already set).
        init_proj(Some("definitely_not_proj_lib"), None);
    }

    /// 候选顺序：显式 > 环境变量 > 运行目录（旧行为）> 可执行文件旁（安装布局）。
    #[test]
    fn proj_candidates_are_ordered() {
        let exe = Path::new("bin");
        assert_eq!(
            lib_candidates(None, Some("C:/proj/proj_9.dll"), Some(exe), "proj_9.dll"),
            vec![
                PathBuf::from("C:/proj/proj_9.dll"),
                PathBuf::from("proj_9.dll"),
                exe.join("proj_9.dll"),
            ]
        );
        // 显式参数独占，不再尝试其它候选。
        assert_eq!(
            lib_candidates(Some("x.dll"), Some("y.dll"), Some(exe), "proj_9.dll"),
            vec![PathBuf::from("x.dll")]
        );

        let cwd = Path::new("work");
        assert_eq!(
            data_candidates(None, None, Some(cwd), Some(exe)),
            vec![
                cwd.join("share").join("proj"),
                exe.join("share").join("proj"),
                exe.join("..").join("share").join("proj"),
            ]
        );
        assert_eq!(
            data_candidates(None, Some("D:/projdata"), Some(cwd), None)[0],
            PathBuf::from("D:/projdata")
        );
        // PROJ_LIB 不参与解析（避免系统 PostGIS 旧 proj.db 被误用）。
        assert!(data_candidates(None, None, None, None).is_empty());
    }
}
