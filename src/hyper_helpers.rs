use hyper::{Body, Request, Response};
use hyper::body::Bytes;
use futures_util::future::Future;

/// Convert `Request/Response` body from `Body` to `Bytes`.
///
/// It's intended to use with `map_request_body` or `map_response_body`.
pub async fn body_to_bytes(body: Body) -> Result<Bytes, hyper::Error> {
    hyper::body::to_bytes(body).await
}

/// Convert `Request/Response` body from `Bytes` to `Body`.
///
/// It's intended to use with `map_request_body` or `map_response_body`.
pub async fn bytes_to_body(bytes: Bytes) -> Result<Body, hyper::Error> {
    Ok(Body::from(bytes))
}

/// Map `Request` body.
///
/// Standard `Body` is a `Stream` so this function is `async` to allow to aggregate `Stream` to vectors.
pub async fn map_request_body<T, U, F, FO>(req: Request<T>, mapper: F) -> Result<Request<U>, hyper::Error>
    where
        FO: Future<Output = Result<U, hyper::Error>>,
        F: FnOnce(T) -> FO
{
    let (parts, body) = req.into_parts();
    let mapped_body = mapper(body).await?;
    Ok(Request::from_parts(parts, mapped_body))
}

/// Map `Response` body.
///
/// Standard `Body` is a `Stream` so this function is `async` to allow to aggregate `Stream` to vectors.
pub async fn map_response_body<T, U, F, FO>(req: Response<T>, mapper: F) -> Result<Response<U>, hyper::Error>
    where
        FO: Future<Output = Result<U, hyper::Error>>,
        F: FnOnce(T) -> FO
{
    let (parts, body) = req.into_parts();
    let mapped_body = mapper(body).await?;
    Ok(Response::from_parts(parts, mapped_body))
}

/// Clone `Response`.
///
/// _Warning:_: Extensions cannot be cloned.
pub fn clone_response<T: Clone>(response: &Response<T>) -> Response<T> {
    let mut new_resp = Response::new(response.body().clone());
    *new_resp.status_mut() = response.status().clone();
    *new_resp.version_mut() = response.version().clone();
    *new_resp.headers_mut() = response.headers().clone();
    // *new_resp.extensions_mut() = response.extensions().clone();
    new_resp
}

/// Consumes `Response<Body>` and returns result with the original `Response<Body>`
/// and cloned `Response<Bytes>`.
pub async fn fork_response(response: Response<Body>)
    -> Result<(Response<Body>, Response<Bytes>), hyper::Error>
{
    // We need to convert the body to bytes to clone the response.
    let response_with_byte_body = map_response_body(
        response, body_to_bytes
    ).await?;
    // And then clone it and convert back to `Body` so we can return it.
    let response = map_response_body(
        clone_response(&response_with_byte_body),
        bytes_to_body
    ).await?;
    Ok((response, response_with_byte_body))
}
