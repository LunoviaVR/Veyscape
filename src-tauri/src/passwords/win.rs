use crate::errors::{AppError, AppResult};
use windows::core::{HSTRING, PCWSTR};
use windows::Security::Credentials::UI::{
    UserConsentVerificationResult, UserConsentVerifier, UserConsentVerifierAvailability,
};
use windows::Win32::Foundation::{LocalFree, HLOCAL, HWND};
use windows::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};
use windows::Win32::System::WinRT::IUserConsentVerifierInterop;
use windows_future::IAsyncOperation;

// Not a secret; it only means a generic DPAPI dump of the database does not decrypt it as is.
const ENTROPY: &[u8] = b"veyscape-logins-v1";

fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    }
}

/// Copies a DPAPI output buffer and frees it.
fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    if out.pbData.is_null() {
        return Vec::new();
    }
    let v = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
    unsafe { LocalFree(Some(HLOCAL(out.pbData.cast()))) };
    v
}

/// Encrypts with the current Windows user's DPAPI key: only this user account can decrypt it.
pub fn protect(plain: &str) -> AppResult<Vec<u8>> {
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptProtectData(
            &blob(plain.as_bytes()),
            PCWSTR::null(),
            Some(&blob(ENTROPY)),
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )?
    };
    Ok(take(out))
}

pub fn unprotect(data: &[u8]) -> AppResult<String> {
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptUnprotectData(
            &blob(data),
            None,
            Some(&blob(ENTROPY)),
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )
    }
    .map_err(|_| AppError::Engine("this saved password cannot be decrypted".into()))?;
    String::from_utf8(take(out))
        .map_err(|_| AppError::Engine("this saved password cannot be decrypted".into()))
}

/// Asks Windows Hello (PIN, face or fingerprint) to confirm the user, over `window`. Blocks until
/// answered, so never call it on the UI thread. Without Windows Hello set up there is nothing to
/// ask and the answer is yes (Chrome asks for the account password then; not implemented).
pub fn verify_user(window: &tauri::Window, message: &str) -> AppResult<bool> {
    match UserConsentVerifier::CheckAvailabilityAsync()?.join()? {
        UserConsentVerifierAvailability::Available => {}
        UserConsentVerifierAvailability::DeviceBusy => {
            return Err(AppError::Denied("Windows Hello is busy. Try again.".into()))
        }
        _ => return Ok(true),
    }
    // Tauri may link a different `windows` version; pass the handle as a plain pointer.
    let hwnd = HWND(window.hwnd()?.0 as _);
    let interop = windows::core::factory::<UserConsentVerifier, IUserConsentVerifierInterop>()?;
    let op: IAsyncOperation<UserConsentVerificationResult> =
        unsafe { interop.RequestVerificationForWindowAsync(hwnd, &HSTRING::from(message))? };
    Ok(op.join()? == UserConsentVerificationResult::Verified)
}
