# Addon Proxy

```bash
cargo run
```

## Development

Routes to published addons:

```toml
# proxy_config.toml

[[routes]]
from = "stremio-addon-proxy.herokuapp.com/helloworld"
to = "https://stremio-addon-helloworld.herokuapp.com"

[[routes]]
from = "stremio-addon-proxy.herokuapp.com/rust-addon"
to = "https://stremio-addon-example.herokuapp.com"
```
 
_Notes:_ 
  - Deployed addons and the proxy on Heroku may be broken for testing purposes. 
  - The first response may be slow because of Heroku dyno cold start.
  - The proxy may have non-standard configuration - e.g. disabled cache.
  - Addons and the proxy is currently deployed manually by Heroku CLI.
 
## How it works

### 1. Layer - The Proxy core

1. Proxy is created and then started in `main.rs`.
1. New proxy requires callback `on_request` and HTTP client that will be sent to `on_request`.
1. The most important code is in `proxy.rs` - `Proxy::start`:
   1. The proxy tries to load `ProxyConfig`and open database.
   1. The proxy creates channel(s) for communication between the core and `on_request` callbacks 
      (it's useful e.g. for `ProxyConfig` reloading though API calls).
   1. The server is started.
   
### 2. Layer - Middlewares

1. The most important function in this layer is `on_request` (currently in `lib.rs`).
1. `on_request` receives user's request from the proxy core and then:
   1. The request is passed into middleware pipeline (function `apply_request_middlewares`).
   1. Middlewares return modified request or custom / error / cached response. 
      Middlewares may invoke side-effects like the cache reloading during their execution.
   1. If the pipeline result is a response, then the response is returned by the proxy server.
   1. If the pipeline result is a request, then the request is sent and 
      a successful response is cached and returned by the proxy server.
      
### 3. Layer - Business rules

1. The rules extend the second layer - e.g. how long should be responses cached.
1. WIP
