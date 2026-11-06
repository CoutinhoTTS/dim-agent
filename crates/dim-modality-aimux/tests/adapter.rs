//! The adapter against a scripted transport: the exact request each operation
//! sends, the files it writes, and what a failure may say.

use aimux_provider_utils::{Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse};
use async_trait::async_trait;
use bytes::Bytes;
use dim_agent_capabilities::host::{
    HttpModelConnection, ModalityConnection, ModelConnection, ModelNetwork, ModelProtocol,
    ResolvedModelConnection,
};
use dim_agent_types::ModelId;
use dim_modality::{AdapterErrorCode, Job, JobFile, ModalityAdapter, ProgressSink};
use dim_modality_aimux::MediaAdapter;
use http::{HeaderMap, Method, StatusCode};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

const CREDENTIAL: &str = "test-credential";

/// One request the adapter sent.
#[derive(Clone, Debug)]
struct Recorded {
    method: Method,
    url: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Recorded {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
    fn body_text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// A reply a responder produces.
struct Reply {
    status: StatusCode,
    body: Vec<u8>,
}

fn ok_json(body: Value) -> Reply {
    Reply {
        status: StatusCode::OK,
        body: serde_json::to_vec(&body).unwrap(),
    }
}

/// A 401 whose body names the credential and a URL, as a provider's does.
fn unauthorized() -> Reply {
    Reply {
        status: StatusCode::UNAUTHORIZED,
        body: format!(
            r#"{{"error":{{"message":"unauthorized: the key {CREDENTIAL} is not valid for https://api.example.test/v1?key={CREDENTIAL}","detail":"BODY-MARKER-7731"}}}}"#
        )
        .into_bytes(),
    }
}

#[derive(Clone)]
struct Scripted {
    requests: Arc<Mutex<Vec<Recorded>>>,
    respond: Arc<dyn Fn(&Recorded) -> Reply + Send + Sync>,
}

#[async_trait]
impl Fetch for Scripted {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
        let recorded = Recorded {
            method: request.method.clone(),
            url: request.url.to_string(),
            headers: request
                .headers
                .iter()
                .map(|(name, value)| {
                    (
                        name.as_str().to_owned(),
                        value.to_str().unwrap_or_default().to_owned(),
                    )
                })
                .collect(),
            body: request.body.to_vec(),
        };
        self.requests.lock().unwrap().push(recorded.clone());
        let reply = (self.respond)(&recorded);
        Ok(FetchResponse::from_bytes(
            reply.status,
            HeaderMap::new(),
            request.url,
            Bytes::from(reply.body),
        ))
    }
}

/// An adapter whose transport answers `respond`, with every request recorded.
fn scripted(
    respond: impl Fn(&Recorded) -> Reply + Send + Sync + 'static,
) -> (Arc<MediaAdapter>, Arc<Mutex<Vec<Recorded>>>) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let fetch: FetchFunction = Arc::new(Scripted {
        requests: requests.clone(),
        respond: Arc::new(respond),
    });
    (Arc::new(MediaAdapter::scripted(fetch)), requests)
}

fn connection(base_url: &str, model: &str) -> ModalityConnection {
    ModalityConnection {
        resolved: ResolvedModelConnection {
            connection: ModelConnection::Http(HttpModelConnection {
                protocol: ModelProtocol::OpenaiCompatible,
                base_url: base_url.to_owned(),
                network: ModelNetwork::default(),
                headers: Default::default(),
                bearer: Some(CREDENTIAL.to_owned()),
                api_key: None,
                preset: None,
            }),
            failure_policy: None,
            recovery: None,
        },
        protocol: ModelProtocol::OpenaiCompatible,
        model: model.to_owned(),
    }
}

fn work_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "dim-modality-aimux-{name}-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn input(dir: &Path, name: &str, media_type: &str, bytes: &[u8]) -> JobFile {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    JobFile {
        name: name.to_owned(),
        media_type: media_type.to_owned(),
        path,
    }
}

fn job(
    provider: &str,
    model: &str,
    category: &str,
    operation: &str,
    params: Value,
    files: Vec<JobFile>,
    work_dir: &Path,
) -> Job {
    Job {
        task_id: uuid::Uuid::new_v4().to_string(),
        category: category.to_owned(),
        operation: operation.to_owned(),
        model: ModelId {
            provider_id: provider.to_owned(),
            model_id: model.to_owned(),
        },
        params,
        files,
        work_dir: work_dir.to_path_buf(),
    }
}

/// The 8-byte signature and IHDR length of a PNG, which is all the media-type
/// sniffing reads.
const PNG_HEADER: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";

fn base64(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Runs a blocking operation end to end through the adapter.
async fn run(
    adapter: &Arc<MediaAdapter>,
    job: Job,
    base_url: &str,
    model: &str,
) -> Result<dim_modality::AdapterOutput, dim_modality::AdapterError> {
    let task_id = job.task_id.clone();
    adapter
        .submit(job, connection(base_url, model), CancellationToken::new())
        .await?;
    adapter
        .wait(
            &task_id,
            None,
            connection(base_url, model),
            ProgressSink::none(),
            CancellationToken::new(),
        )
        .await
}

#[tokio::test]
async fn image_generate_pins_the_request_and_names_the_output() {
    let dir = work_dir("image-generate");
    let png = [PNG_HEADER, b"image-bytes"].concat();
    let reply_png = png.clone();
    let (adapter, requests) = scripted(move |_| {
        ok_json(json!({"created": 1, "data": [{"b64_json": base64(&reply_png)}]}))
    });
    let output = run(
        &adapter,
        job(
            "openai",
            "gpt-image-1",
            "image",
            "generate",
            json!({"prompt": "a cat", "n": 1, "size": "1024x1024", "outputFormat": "png"}),
            Vec::new(),
            &dir,
        ),
        "https://api.openai.com/v1",
        "gpt-image-1",
    )
    .await
    .unwrap();

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, Method::POST);
    assert_eq!(request.url, "https://api.openai.com/v1/images/generations");
    assert_eq!(
        request.header("authorization"),
        Some("Bearer test-credential")
    );
    let body: Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(body["model"], "gpt-image-1");
    assert_eq!(body["prompt"], "a cat");
    assert_eq!(body["n"], 1);
    assert_eq!(body["size"], "1024x1024");
    assert_eq!(body["output_format"], "png");

    assert_eq!(output.files.len(), 1);
    assert_eq!(output.files[0].name, "image-1.png");
    assert_eq!(output.files[0].media_type, "image/png");
    assert_eq!(std::fs::read(&output.files[0].path).unwrap(), png);
    assert!(path_is_inside(&output.files[0].path, &dir));
    assert!(output.data.is_none());
}

#[tokio::test]
async fn image_edit_pins_the_multipart_fields() {
    let dir = work_dir("image-edit");
    let input = input(
        &dir,
        "cat.png",
        "image/png",
        &[PNG_HEADER, b"input"].concat(),
    );
    let (adapter, requests) = scripted(move |_| {
        ok_json(json!({"created": 1, "data": [{"b64_json": base64(PNG_HEADER)}]}))
    });
    run(
        &adapter,
        job(
            "openai",
            "gpt-image-1",
            "image",
            "edit",
            json!({"prompt": "add a hat"}),
            vec![input],
            &dir,
        ),
        "https://api.openai.com/v1",
        "gpt-image-1",
    )
    .await
    .unwrap();

    let requests = requests.lock().unwrap();
    let request = &requests[0];
    assert_eq!(request.url, "https://api.openai.com/v1/images/edits");
    assert!(
        request
            .header("content-type")
            .is_some_and(|value| value.starts_with("multipart/form-data; boundary="))
    );
    let body = request.body_text();
    for field in ["name=\"model\"", "name=\"prompt\"", "name=\"image\""] {
        assert!(body.contains(field), "multipart body has no {field}");
    }
    assert!(body.contains("add a hat"));
    assert!(body.contains("filename=\"image\""));
    assert!(body.contains("Content-Type: image/png"));
}

#[tokio::test]
async fn a_provider_failure_carries_no_body_no_credential_and_no_url() {
    let dir = work_dir("redaction");
    let (adapter, _requests) = scripted(|_| unauthorized());
    let error = run(
        &adapter,
        job(
            "openai",
            "gpt-image-1",
            "image",
            "generate",
            json!({"prompt": "a cat"}),
            Vec::new(),
            &dir,
        ),
        "https://api.openai.com/v1",
        "gpt-image-1",
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, AdapterErrorCode::ProviderError);
    let message = &error.message;
    assert!(
        !message.contains(CREDENTIAL),
        "message carries the credential: {message}"
    );
    assert!(
        !message.contains("http"),
        "message carries a URL: {message}"
    );
    assert!(
        !message.contains("BODY-MARKER-7731"),
        "message carries the response body: {message}"
    );
    assert!(
        !message.contains('\n'),
        "message is not one line: {message}"
    );
}

#[tokio::test]
async fn an_unmapped_provider_is_unavailable() {
    let dir = work_dir("unmapped");
    let (adapter, _requests) = scripted(|_| ok_json(json!({})));
    let error = run(
        &adapter,
        job(
            "6f1c2f9e-0000-4000-8000-000000000000",
            "anything",
            "image",
            "generate",
            json!({"prompt": "a cat"}),
            Vec::new(),
            &dir,
        ),
        "https://api.example.test/v1",
        "anything",
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, AdapterErrorCode::Unavailable);
}

fn path_is_inside(path: &Path, dir: &Path) -> bool {
    path.strip_prefix(dir).is_ok()
}

#[tokio::test]
async fn video_stores_the_operation_and_downloads_the_result() {
    let dir = work_dir("video");
    let video = b"fake-mp4-bytes-for-the-download".to_vec();
    let (url, downloaded) = video_server(video.clone()).await;
    let polls = Arc::new(Mutex::new(0usize));
    let scripted_polls = polls.clone();
    let reply_url = url.clone();
    let (adapter, requests) = scripted(move |request| {
        if request.method == Method::POST {
            return ok_json(json!({"request_id": "req_1"}));
        }
        let mut count = scripted_polls.lock().unwrap();
        *count += 1;
        if *count == 1 {
            Reply {
                status: StatusCode::ACCEPTED,
                body: br#"{"status":"pending","progress":0.5}"#.to_vec(),
            }
        } else {
            ok_json(json!({
                "status": "done",
                "video": {"url": reply_url, "duration": 5},
            }))
        }
    });

    let task = job(
        "xai",
        "grok-imagine-video",
        "video",
        "generate",
        json!({"prompt": "a cat"}),
        Vec::new(),
        &dir,
    );
    let task_id = task.task_id.clone();
    let submitted = adapter
        .submit(
            task,
            connection("https://api.x.ai/v1", "grok-imagine-video"),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    let handle = submitted.remote_handle.expect("video returns a handle");
    assert!(
        handle.contains("req_1"),
        "handle names the operation: {handle}"
    );
    assert!(handle.contains(&task_id), "handle keeps the job: {handle}");

    let output = adapter
        .wait(
            &task_id,
            Some(&handle),
            connection("https://api.x.ai/v1", "grok-imagine-video"),
            ProgressSink::none(),
            CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(output.files.len(), 1);
    assert_eq!(output.files[0].name, "video-1.mp4");
    assert_eq!(output.files[0].media_type, "video/mp4");
    assert_eq!(std::fs::read(&output.files[0].path).unwrap(), video);

    let requests = requests.lock().unwrap();
    let start: Vec<_> = requests
        .iter()
        .filter(|request| request.method == Method::POST)
        .collect();
    assert_eq!(start.len(), 1, "one start call");
    assert_eq!(start[0].url, "https://api.x.ai/v1/videos/generations");
    assert_eq!(*polls.lock().unwrap(), 2, "two status polls");

    // The download is a different origin: the credential must not travel.
    let downloaded = downloaded.lock().unwrap();
    assert_eq!(downloaded.len(), 1);
    for (name, value) in &downloaded[0] {
        assert!(
            !value.contains(CREDENTIAL),
            "download carried {name}: {value}"
        );
        assert!(
            !matches!(
                name.as_str(),
                "authorization" | "x-goog-api-key" | "xi-api-key" | "x-hume-api-key"
            ),
            "download carried {name}: {value}"
        );
    }
}

/// A credentialed download that meets a cross-origin redirect stops at it:
/// the provider's own header must not cross, and reqwest strips only
/// `Authorization`.
#[tokio::test]
async fn a_credentialed_download_stops_at_a_cross_origin_redirect() {
    use axum::Router;
    use axum::routing::get;

    // Origin B is the redirect's target: it must never see the connection.
    let seen_b = Arc::new(Mutex::new(Vec::new()));
    let recorder_b = seen_b.clone();
    let b_app = Router::new().route(
        "/collect",
        get(move |headers: axum::http::HeaderMap| {
            let recorder = recorder_b.clone();
            async move {
                recorder.lock().unwrap().push(header_pairs(&headers));
                "collected"
            }
        }),
    );
    let b_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let b_base = format!("http://{}", b_listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(b_listener, b_app).await.unwrap();
    });

    // Origin A serves the video URL; it redirects to B.
    let seen_a = Arc::new(Mutex::new(Vec::new()));
    let recorder_a = seen_a.clone();
    let target = format!("{b_base}/collect");
    let a_app = Router::new().route(
        "/download",
        get(move |headers: axum::http::HeaderMap| {
            let recorder = recorder_a.clone();
            let target = target.clone();
            async move {
                recorder.lock().unwrap().push(header_pairs(&headers));
                let mut response = axum::response::Response::new(axum::body::Body::empty());
                *response.status_mut() = StatusCode::FOUND;
                response
                    .headers_mut()
                    .insert(axum::http::header::LOCATION, target.parse().unwrap());
                response
            }
        }),
    );
    let a_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let a_base = format!("http://{}", a_listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(a_listener, a_app).await.unwrap();
    });

    // The connection shares A's origin, so the download carries its headers;
    // a custom provider header is the one reqwest would let cross a redirect.
    let connect = || {
        let mut connection = connection(&format!("{a_base}/v1"), "grok-imagine-video");
        if let ModelConnection::Http(http) = &mut connection.resolved.connection {
            http.headers
                .insert("x-goog-api-key".to_owned(), CREDENTIAL.to_owned());
        }
        connection
    };
    let reply_url = format!("{a_base}/download");
    let (adapter, _requests) = scripted(move |request| {
        if request.method == Method::POST {
            return ok_json(json!({"request_id": "req_1"}));
        }
        ok_json(json!({
            "status": "done",
            "video": {"url": reply_url, "duration": 5},
        }))
    });

    let dir = work_dir("redirect");
    let task = job(
        "xai",
        "grok-imagine-video",
        "video",
        "generate",
        json!({"prompt": "a cat"}),
        Vec::new(),
        &dir,
    );
    let task_id = task.task_id.clone();
    let handle = adapter
        .submit(task, connect(), CancellationToken::new())
        .await
        .unwrap()
        .remote_handle
        .expect("video returns a handle");
    let outcome = adapter
        .wait(
            &task_id,
            Some(&handle),
            connect(),
            ProgressSink::none(),
            CancellationToken::new(),
        )
        .await;
    // The redirect target is the property under test: it must never be asked.
    let seen_b = seen_b.lock().unwrap();
    assert!(
        seen_b.is_empty(),
        "the redirect target was asked: {seen_b:?}"
    );
    drop(seen_b);
    let error = outcome.unwrap_err();
    assert_eq!(error.message, "The video download answered HTTP 302 Found");

    // A was asked with the connection's headers; the redirect never reached B.
    let seen_a = seen_a.lock().unwrap();
    assert_eq!(seen_a.len(), 1);
    assert!(
        seen_a[0]
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("x-goog-api-key")
                && value == CREDENTIAL),
        "the download carried the connection's header: {:?}",
        seen_a[0]
    );
}

/// The headers of one request as (name, value) pairs.
fn header_pairs(headers: &axum::http::HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                value.to_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

/// A tiny HTTP server for one video, recording the headers it was asked with.
async fn video_server(body: Vec<u8>) -> (String, Arc<Mutex<Vec<Vec<(String, String)>>>>) {
    use axum::Router;
    use axum::routing::get;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorder = seen.clone();
    let app = Router::new().route(
        "/video.mp4",
        get(move |headers: axum::http::HeaderMap| {
            let recorder = recorder.clone();
            let body = body.clone();
            async move {
                recorder.lock().unwrap().push(
                    headers
                        .iter()
                        .map(|(name, value)| {
                            (
                                name.as_str().to_owned(),
                                value.to_str().unwrap_or_default().to_owned(),
                            )
                        })
                        .collect(),
                );
                ([(axum::http::header::CONTENT_TYPE, "video/mp4")], body)
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{address}/video.mp4"), seen)
}
