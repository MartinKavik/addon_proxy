use std::str::FromStr;
use stremio_core::types::addons::ResourceRef;
use hyper::{Body, Response};

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

pub fn validate_response(response: &Response<Body>) -> bool {
    response.status().is_success()
}