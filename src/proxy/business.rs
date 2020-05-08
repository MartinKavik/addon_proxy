use std::str::FromStr;
use stremio_core::types::addons::ResourceRef;

pub fn validate_request(path: &str) -> bool {
    if path == "/manifest.json" {
        return true
    }

    if let Err(error) = ResourceRef::from_str(&path) {
        eprintln!("Request validation error! (Path: '{}', Error: '{:#?}')", path, error);
        return false
    }

    true
}