//! Files installed next to the binary in a release archive:
//!
//! ```text
//! accessmod
//! share/proj/proj.db         PROJ database (bundled builds link PROJ statically)
//! share/accessmod/web/       the built web client, served by `accessmod serve`
//! ```
//!
//! A development build finds none of them and uses the system PROJ data and
//! `--web-dir`.

use std::ffi::CString;
use std::path::PathBuf;

/// `share/` next to the running binary.
fn share_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    Some(exe.parent()?.join("share"))
}

/// Point PROJ at the database shipped with the binary, unless `PROJ_DATA`
/// or `PROJ_LIB` names one. Call it once, before any coordinate operation.
/// Returns the directory used, `None` when PROJ keeps its own search path.
pub fn use_shipped_proj_data() -> Option<PathBuf> {
    if std::env::var_os("PROJ_DATA").is_some() || std::env::var_os("PROJ_LIB").is_some() {
        return None;
    }
    let dir = share_dir()?.join("proj");
    if !dir.join("proj.db").is_file() {
        return None;
    }
    let path = CString::new(dir.to_str()?).ok()?;
    let paths = [path.as_ptr(), std::ptr::null()];
    // SAFETY: a null-terminated array of valid C strings, copied by GDAL.
    unsafe { gdal_sys::OSRSetPROJSearchPaths(paths.as_ptr()) };
    Some(dir)
}

/// The web client shipped with the binary, when there is one.
pub fn shipped_web_dir() -> Option<PathBuf> {
    let dir = share_dir()?.join("accessmod").join("web");
    dir.join("index.html").is_file().then_some(dir)
}
