use std::str::FromStr;
use stremio_core::types::addons::ResourceRef;
use hyper::{Body, Response, Request};
use hyper::body::Bytes;

// The proxy returns BAD_REQUEST when the request is invalid 
// and doesn't allow to pass it to the origin.
pub fn validate_request(_: &Request<Bytes>, path: &str) -> bool {
    match path {
        "/manifest.json" | "/" | "" => return true,
        public if public.starts_with("/public") => return true,
        images if images.starts_with("/images") => return true,
        _ => ()
    }

    if let Err(error) = ResourceRef::from_str(&path) {
        eprintln!("Request validation error! (Path: '{}', Error: '{:#?}')", path, error);
        return false
    }
    true
}

/// The proxy doesn't allow to cache an invalid response
/// and tries to return its previous valid cached version. 
pub fn validate_response(response: &Response<Body>) -> bool {
    response.status().is_success()
}
