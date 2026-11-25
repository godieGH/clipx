use super::endpoint::token_path;
use rand::{RngCore, rngs::OsRng};
use std::{
    fs,
    io::{self, Write},
};
use tokio_tungstenite::tungstenite::{
    handshake::server::{ErrorResponse, Request},
    http::StatusCode,
};

/// Fresh token every core start. Must run BEFORE the endpoint is created,
/// so a client that can connect can always read the matching token.
pub fn issue_token(name: &str) -> io::Result<String> {
    let mut b = [0u8; 32];
    OsRng.fill_bytes(&mut b);
    let token = hex::encode(b);

    let path = token_path(name)?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let _ = fs::remove_file(&path); // so the mode below applies to a new file
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(&path)?.write_all(token.as_bytes())?;
    Ok(token)
}

pub fn remove_token(name: &str) {
    if let Ok(p) = token_path(name) {
        let _ = fs::remove_file(p);
    }
}

// ErrorResponse is tungstenite's handshake error type; its size is not ours to change.
#[allow(clippy::result_large_err)]
pub fn check(req: &Request, token: &str) -> Result<(), ErrorResponse> {
    let ok = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|got| ct_eq(got.as_bytes(), token.as_bytes()));
    if ok {
        return Ok(());
    }
    let mut r = ErrorResponse::new(Some("unauthorized".to_string()));
    *r.status_mut() = StatusCode::UNAUTHORIZED;
    Err(r)
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

#[cfg(windows)]
pub fn current_user_sd()
-> io::Result<interprocess::os::windows::security_descriptor::SecurityDescriptor> {
    use interprocess::os::windows::security_descriptor::SecurityDescriptor;
    use widestring::U16CString;
    // Protected DACL, one ACE: generic-all for this user's SID only.
    let sddl = format!("D:P(A;;GA;;;{})", current_user_sid_string()?);
    let w = U16CString::from_str(&sddl).map_err(io::Error::other)?;
    SecurityDescriptor::deserialize(&w)
}

#[cfg(windows)]
fn current_user_sid_string() -> io::Result<String> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE, LocalFree},
        Security::{
            Authorization::ConvertSidToStringSidW, GetTokenInformation, TOKEN_QUERY, TOKEN_USER,
            TokenUser,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut len = 0u32;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut len);
        let mut buf = vec![0u64; (len as usize).div_ceil(8)]; // u64 = pointer-aligned
        let ok = GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len);
        CloseHandle(token);
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut s: *mut u16 = std::ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut s) == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut n = 0;
        while *s.add(n) != 0 {
            n += 1;
        }
        let out = String::from_utf16_lossy(std::slice::from_raw_parts(s, n));
        LocalFree(s.cast());
        Ok(out)
    }
}
