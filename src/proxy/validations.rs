use hyper::body::Bytes;
use hyper::{Body, Request, Response};
use std::str::FromStr;
use stremio_core::types::addons::ResourceRef;

// The proxy returns BAD_REQUEST when the request is invalid
// and doesn't allow to pass it to the origin.
pub fn validate_request(_: &Request<Bytes>, path: &str) -> bool {
    match path {
        "/manifest.json" | "/" | "" => return true,
        public if public.starts_with("/public") => return true,
        images if images.starts_with("/images") => return true,
        _ => (),
    }

    if let Err(error) = ResourceRef::from_str(path) {
        eprintln!(
            "Request validation error! (Path: '{}', Error: '{:#?}')",
            path, error
        );
        return false;
    }
    true
}

/// The proxy doesn't allow to cache an invalid response
/// and tries to return its previous valid cached version.
pub fn validate_response(response: &Response<Body>) -> bool {
    response.status().is_success()
}

// ------ ------- TESTS ------ ------

#[cfg(test)]
mod tests {
    use super::*;
    use http::StatusCode;

    // ------ validate_request ------

    #[test]
    fn validate_request_manifest() {
        let request = Request::default();
        let path = "/manifest.json";
        assert!(validate_request(&request, path));
    }

    #[test]
    fn validate_request_root() {
        let request = Request::default();
        let path = "";
        assert!(validate_request(&request, path));
    }

    #[test]
    fn validate_request_root_slash() {
        let request = Request::default();
        let path = "/";
        assert!(validate_request(&request, path));
    }

    #[test]
    fn validate_request_public() {
        let request = Request::default();
        let path = "/public/docs/file.pdf";
        assert!(validate_request(&request, path));
    }

    #[test]
    fn validate_request_images() {
        let request = Request::default();
        let path = "/images/my_image.png";
        assert!(validate_request(&request, path));
    }

    #[test]
    fn validate_request_top() {
        let request = Request::default();
        let path = "/catalog/movie/top.json";
        assert!(validate_request(&request, path));
    }

    #[test]
    fn validate_request_unknown() {
        let request = Request::default();
        let path = "/unknown";
        assert!(!validate_request(&request, path));
    }

    // ------ validate_response ------

    #[test]
    fn validate_response_success() {
        let mut response = Response::default();
        *response.status_mut() = StatusCode::OK;
        assert!(validate_response(&response));
    }

    #[test]
    fn validate_response_fail() {
        let mut response = Response::default();
        *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        assert!(!validate_response(&response));
    }
}
