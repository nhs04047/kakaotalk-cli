// Linux/other: stub backend that panics at runtime
// This allows the crate to compile on any platform for CI/checking.

use crate::*;
use kakaocli_core::model::DbKey;

pub struct StubBackend;

impl PlatformBackend for StubBackend {
    fn resolve_db_key() -> Result<DbKey, PlatformError> {
        Err(PlatformError::Other(
            "kakaocli is only supported on macOS and Windows".into()
        ))
    }

    fn check_status() -> Result<AppStatus, PlatformError> {
        Err(PlatformError::Other(
            "kakaocli is only supported on macOS and Windows".into()
        ))
    }

    fn login(_email: &str, _password: &str) -> Result<(), PlatformError> {
        Err(PlatformError::Other(
            "kakaocli is only supported on macOS and Windows".into()
        ))
    }

    fn send_message(_chat_name: &str, _text: &str) -> Result<(), PlatformError> {
        Err(PlatformError::Other(
            "kakaocli is only supported on macOS and Windows".into()
        ))
    }
}