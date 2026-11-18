//! API keys are separate from settings and encrypted for the current Windows user.
#[cfg(windows)]
use anyhow::Context;
use anyhow::{bail, Result};

fn path(provider: &str) -> Result<std::path::PathBuf> {
    if ![
        "lm_studio",
        "ollama",
        "open_router",
        "open_ai",
        "compatible",
        "fish_audio",
    ]
    .contains(&provider)
    {
        bail!("Неизвестный провайдер ключа");
    }
    Ok(crate::config::data_dir()
        .join("credentials")
        .join(format!("{provider}.bin")))
}

pub fn save(provider: &str, key: &str) -> Result<()> {
    let path = path(provider)?;
    let key = key.trim();
    if key.len() > 4096 || key.contains(['\r', '\n']) {
        bail!("Некорректный API-ключ");
    }
    if key.is_empty() {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        return Ok(());
    }
    let encrypted = protect(key.as_bytes(), false)?;
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(path, encrypted)?;
    Ok(())
}

pub fn load(provider: &str) -> Result<Option<String>> {
    let path = path(provider)?;
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8(protect(&std::fs::read(path)?, true)?)?
            .trim()
            .into(),
    ))
}

pub fn present(provider: &str) -> bool {
    path(provider).is_ok_and(|p| p.is_file())
}

#[cfg(windows)]
fn protect(bytes: &[u8], decrypt: bool) -> Result<Vec<u8>> {
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::{LocalFree, HLOCAL},
            Security::Cryptography::{
                CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
            },
        },
    };
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        if decrypt {
            CryptUnprotectData(
                &input,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptProtectData(
                &input,
                PCWSTR::null(),
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
        .context("Windows не смогла обработать сохранённый API-ключ")?;
        let value = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
        Ok(value)
    }
}

#[cfg(not(windows))]
fn protect(_: &[u8], _: bool) -> Result<Vec<u8>> {
    bail!("Хранение API-ключей поддерживается в Windows через DPAPI")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credential_names_cannot_escape_their_directory() {
        assert!(path("../../settings").is_err());
    }
    #[cfg(windows)]
    #[test]
    fn dpapi_roundtrip_is_encrypted_for_the_current_user() {
        let encrypted = protect(b"unit-test-key", false).unwrap();
        assert_ne!(encrypted, b"unit-test-key");
        assert_eq!(protect(&encrypted, true).unwrap(), b"unit-test-key");
    }
}
