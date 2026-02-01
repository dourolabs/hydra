pub mod config;
pub mod s3;

use anyhow::Result;
use axum::{Json, Router, routing::get};
use http::{Method, Request, Response, StatusCode, Uri, header};
use http_body::Body as HttpBody;
use pin_project_lite::pin_project;
use serde_json::json;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::pin::Pin;
use std::task::{Context, Poll, ready};
use tokio::net::TcpListener;
use tower::{Layer, Service};
use tower_http::{limit::RequestBodyLimitLayer, trace::TraceLayer};
use tracing::{error, info};

pub fn build_router(storage_root: PathBuf, request_body_limit_bytes: usize) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .merge(s3::router(storage_root))
        .layer((
            TraceLayer::new_for_http(),
            RequestBodyLimitLoggerLayer::new(request_body_limit_bytes),
            RequestBodyLimitLayer::new(request_body_limit_bytes),
        ))
}

pub async fn serve(
    listener: TcpListener,
    storage_root: PathBuf,
    request_body_limit_bytes: usize,
) -> Result<()> {
    axum::serve(
        listener,
        build_router(storage_root, request_body_limit_bytes),
    )
    .await?;
    Ok(())
}

async fn healthz() -> Json<serde_json::Value> {
    info!("healthz invoked");
    Json(json!({ "status": "ok" }))
}

#[derive(Clone, Copy)]
struct RequestBodyLimitLoggerLayer {
    limit_bytes: usize,
}

impl RequestBodyLimitLoggerLayer {
    fn new(limit_bytes: usize) -> Self {
        Self { limit_bytes }
    }
}

impl<S> Layer<S> for RequestBodyLimitLoggerLayer {
    type Service = RequestBodyLimitLogger<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RequestBodyLimitLogger {
            inner,
            limit_bytes: self.limit_bytes,
        }
    }
}

#[derive(Clone)]
struct RequestBodyLimitLogger<S> {
    inner: S,
    limit_bytes: usize,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for RequestBodyLimitLogger<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
    ResBody: HttpBody,
{
    type Response = Response<ResBody>;
    type Error = S::Error;
    type Future = RequestBodyLimitLoggerFuture<S::Future, ResBody>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let metadata = RequestMetadata::new(&req, self.limit_bytes);
        let mut future = RequestBodyLimitLoggerFuture {
            inner: self.inner.call(req),
            limit_bytes: self.limit_bytes,
            metadata,
            logged: false,
            _marker: PhantomData,
        };

        if future.metadata.exceeded_at_request {
            future.logged = true;
            log_request_body_limit(&future.metadata, self.limit_bytes, LogPhase::RequestHeader);
        }

        future
    }
}

pin_project! {
    struct RequestBodyLimitLoggerFuture<F, ResBody> {
        #[pin]
        inner: F,
        limit_bytes: usize,
        metadata: RequestMetadata,
        logged: bool,
        _marker: PhantomData<fn() -> ResBody>,
    }
}

impl<F, ResBody, E> std::future::Future for RequestBodyLimitLoggerFuture<F, ResBody>
where
    F: std::future::Future<Output = Result<Response<ResBody>, E>>,
    ResBody: HttpBody,
{
    type Output = Result<Response<ResBody>, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        let response = ready!(this.inner.as_mut().poll(cx));

        match response {
            Ok(response) => {
                if !*this.logged && is_limit_response(&response) {
                    log_request_body_limit(this.metadata, *this.limit_bytes, LogPhase::Response);
                    *this.logged = true;
                }
                Poll::Ready(Ok(response))
            }
            Err(err) => Poll::Ready(Err(err)),
        }
    }
}

#[derive(Clone)]
struct RequestMetadata {
    method: Method,
    uri: Uri,
    content_length: Option<u64>,
    exceeded_at_request: bool,
}

impl RequestMetadata {
    fn new<ReqBody>(req: &Request<ReqBody>, limit_bytes: usize) -> Self {
        let content_length = req
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());
        let exceeded_at_request = content_length
            .map(|len| len as usize > limit_bytes)
            .unwrap_or(false);

        Self {
            method: req.method().clone(),
            uri: req.uri().clone(),
            content_length,
            exceeded_at_request,
        }
    }
}

#[derive(Clone, Copy)]
enum LogPhase {
    RequestHeader,
    Response,
}

impl LogPhase {
    fn as_str(&self) -> &'static str {
        match self {
            Self::RequestHeader => "request_header",
            Self::Response => "response",
        }
    }
}

fn log_request_body_limit(metadata: &RequestMetadata, limit_bytes: usize, phase: LogPhase) {
    if let Some(content_length) = metadata.content_length {
        error!(
            method = %metadata.method,
            uri = %metadata.uri,
            content_length,
            limit_bytes,
            phase = phase.as_str(),
            "request body limit exceeded"
        );
    } else {
        error!(
            method = %metadata.method,
            uri = %metadata.uri,
            limit_bytes,
            phase = phase.as_str(),
            "request body limit exceeded"
        );
    }
}

fn is_limit_response<B>(response: &Response<B>) -> bool {
    if response.status() != StatusCode::PAYLOAD_TOO_LARGE {
        return false;
    }

    #[allow(clippy::declare_interior_mutable_const)]
    const LIMIT_CONTENT_TYPE: header::HeaderValue =
        header::HeaderValue::from_static("text/plain; charset=utf-8");

    response
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|value| value == LIMIT_CONTENT_TYPE)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use std::sync::{Arc, Mutex};
    use tower::{layer::Layer, service_fn, ServiceExt};
    use tracing::subscriber;
    use tracing_subscriber::fmt::MakeWriter;

    #[derive(Clone)]
    struct BufferingWriter {
        buffer: Arc<Mutex<Vec<u8>>>,
    }

    impl std::io::Write for BufferingWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.buffer.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[derive(Clone)]
    struct BufferingMakeWriter {
        buffer: Arc<Mutex<Vec<u8>>>,
    }

    impl<'a> MakeWriter<'a> for BufferingMakeWriter {
        type Writer = BufferingWriter;

        fn make_writer(&'a self) -> Self::Writer {
            BufferingWriter {
                buffer: Arc::clone(&self.buffer),
            }
        }
    }

    #[tokio::test]
    async fn logs_when_content_length_exceeds_limit() {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let make_writer = BufferingMakeWriter {
            buffer: Arc::clone(&buffer),
        };

        let subscriber = tracing_subscriber::fmt()
            .with_writer(make_writer)
            .with_ansi(false)
            .without_time()
            .finish();
        let _guard = subscriber::set_default(subscriber);

        let service = service_fn(|_: Request<Body>| async move {
            Ok::<_, std::convert::Infallible>(Response::new(Body::empty()))
        });
        let mut service = RequestBodyLimitLoggerLayer::new(5).layer(service);

        let request = Request::builder()
            .method(Method::POST)
            .uri("/upload")
            .header(header::CONTENT_LENGTH, "10")
            .body(Body::from(vec![0_u8; 10]))
            .unwrap();

        let result = service.ready().await.unwrap().call(request).await;

        assert!(result.is_ok());

        let logs = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
        assert!(logs.contains("request body limit exceeded"));
        assert!(logs.contains("uri=/upload"));
    }
}
