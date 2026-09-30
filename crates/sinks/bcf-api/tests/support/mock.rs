//! A small in-process BCF API 3.0 server for tests: plain HTTP on a local
//! port, topics, comments and viewpoints in memory, OAuth2 client
//! credentials and device flow. Never a real server.
#![allow(dead_code, clippy::missing_panics_doc)]

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

/// The project every test server holds.
pub const PROJECT: &str = "p-1";
/// The client credentials the server accepts.
pub const CLIENT_ID: &str = "axioval-test";
pub const CLIENT_SECRET: &str = "s3cret";
/// The token it issues.
pub const TOKEN: &str = "t0ken";
/// Who the server says wrote what the client posts.
pub const USER: &str = "api.user@example.com";
/// When the server says it happened.
pub const NOW: &str = "2026-09-30T10:00:00Z";

/// What the server holds.
#[derive(Default)]
pub struct State {
    /// Topics in creation order.
    pub topics: Vec<Value>,
    pub comments: BTreeMap<String, Vec<Value>>,
    pub viewpoints: BTreeMap<String, Vec<Value>>,
    /// Every request as `METHOD path`.
    pub requests: Vec<String>,
    /// Token polls of the device flow so far.
    pub device_polls: usize,
}

/// A running server; stops with the test process.
pub struct Mock {
    pub url: String,
    pub state: Arc<Mutex<State>>,
}

impl Mock {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(State::default()));
        let (shared, base) = (Arc::clone(&state), url.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let (state, base) = (Arc::clone(&shared), base.clone());
                std::thread::spawn(move || serve(stream, &state, &base));
            }
        });
        Self { url, state }
    }

    /// A reviewer changes a topic's status on the server.
    pub fn set_status(&self, guid: &str, status: &str) {
        let mut state = self.state.lock().unwrap();
        let topic = state
            .topics
            .iter_mut()
            .find(|topic| topic["guid"] == guid)
            .unwrap();
        topic["topic_status"] = status.into();
        topic["modified_author"] = "C. Reviewer".into();
        topic["modified_date"] = "2026-09-30T12:00:00Z".into();
    }

    pub fn topic_count(&self) -> usize {
        self.state.lock().unwrap().topics.len()
    }

    pub fn topic(&self, guid: &str) -> Value {
        let state = self.state.lock().unwrap();
        state
            .topics
            .iter()
            .find(|topic| topic["guid"] == guid)
            .unwrap()
            .clone()
    }

    pub fn comments(&self, guid: &str) -> Vec<Value> {
        let state = self.state.lock().unwrap();
        state.comments.get(guid).cloned().unwrap_or_default()
    }

    pub fn viewpoint_count(&self) -> usize {
        let state = self.state.lock().unwrap();
        state.viewpoints.values().map(Vec::len).sum()
    }
}

struct Request {
    method: String,
    path: String,
    authorized: bool,
    body: Vec<u8>,
}

fn read(stream: &TcpStream) -> Option<Request> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_owned();
    let path = parts.next()?.to_owned();
    let (mut length, mut authorized) = (0, false);
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).ok()?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        let (name, value) = header.split_once(':')?;
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => length = value.trim().parse().ok()?,
            "authorization" => authorized = value.trim() == format!("Bearer {TOKEN}"),
            _ => {}
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    Some(Request {
        method,
        path,
        authorized,
        body,
    })
}

fn serve(mut stream: TcpStream, state: &Mutex<State>, base: &str) {
    let Some(request) = read(&stream) else {
        return;
    };
    let (status, body) = route(&request, state, base);
    let text = body.to_string();
    let reason = match status {
        200 => "OK",
        201 => "Created",
        400 => "Bad Request",
        401 => "Unauthorized",
        _ => "Not Found",
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
        text.len()
    );
}

fn form(body: &[u8]) -> BTreeMap<String, String> {
    String::from_utf8_lossy(body)
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(key, value)| (decode(key), decode(value)))
        .collect()
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => out.push(b' '),
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap();
                out.push(u8::from_str_radix(hex, 16).unwrap());
                index += 2;
            }
            byte => out.push(byte),
        }
        index += 1;
    }
    String::from_utf8(out).unwrap()
}

#[allow(clippy::too_many_lines)]
fn route(request: &Request, state: &Mutex<State>, base: &str) -> (u16, Value) {
    let mut state = state.lock().unwrap();
    state
        .requests
        .push(format!("{} {}", request.method, request.path));
    let segments: Vec<&str> = request.path.trim_matches('/').split('/').collect();
    let body = || serde_json::from_slice::<Value>(&request.body).unwrap_or(Value::Null);
    match (request.method.as_str(), segments.as_slice()) {
        ("GET", ["bcf", "versions"]) => (
            200,
            json!({"versions": [{"version_id": "3.0", "detailed_version": "https://github.com/buildingSMART/BCF-API"}]}),
        ),
        ("GET", ["bcf", "3.0", "auth"]) => (
            200,
            json!({"oauth2_token_url": format!("{base}/token"),
                   "supported_oauth2_flows": ["client_credentials"]}),
        ),
        ("POST", ["device"]) => (
            200,
            json!({"device_code": "dev-1", "user_code": "ABCD-EFGH",
                   "verification_uri": format!("{base}/verify"), "expires_in": 60, "interval": 0}),
        ),
        ("POST", ["token"]) => {
            let form = form(&request.body);
            let get = |key: &str| form.get(key).map(String::as_str);
            match get("grant_type") {
                Some("client_credentials")
                    if get("client_id") == Some(CLIENT_ID)
                        && get("client_secret") == Some(CLIENT_SECRET) =>
                {
                    (200, json!({"access_token": TOKEN, "token_type": "bearer"}))
                }
                Some("urn:ietf:params:oauth:grant-type:device_code")
                    if get("device_code") == Some("dev-1") =>
                {
                    state.device_polls += 1;
                    if state.device_polls < 2 {
                        (400, json!({"error": "authorization_pending"}))
                    } else {
                        (200, json!({"access_token": TOKEN, "token_type": "bearer"}))
                    }
                }
                _ => (400, json!({"error": "invalid_client"})),
            }
        }
        (_, ["bcf", "3.0", ..]) if !request.authorized => (401, json!({"message": "sign in"})),
        ("GET", ["bcf", "3.0", "projects"]) => (
            200,
            json!([{"project_id": PROJECT, "name": "Test project"}]),
        ),
        (method, ["bcf", "3.0", "projects", project, "topics", rest @ ..])
            if *project == PROJECT =>
        {
            match (method, rest) {
                ("GET", []) => (200, Value::Array(state.topics.clone())),
                ("POST", []) => {
                    let mut topic = body();
                    let guid = topic["guid"].as_str().unwrap().to_owned();
                    if state
                        .topics
                        .iter()
                        .any(|held| held["guid"] == guid.as_str())
                    {
                        return (400, json!({"message": "exists"}));
                    }
                    topic["creation_author"] = USER.into();
                    topic["creation_date"] = NOW.into();
                    state.topics.push(topic.clone());
                    (201, topic)
                }
                ("GET", [guid]) => state
                    .topics
                    .iter()
                    .find(|topic| topic["guid"] == *guid)
                    .map_or((404, json!({})), |topic| (200, topic.clone())),
                ("PUT", [guid]) => {
                    let Some(held) = state.topics.iter_mut().find(|t| t["guid"] == *guid) else {
                        return (404, json!({}));
                    };
                    let mut topic = body();
                    topic["guid"] = (*guid).into();
                    topic["creation_author"] = held["creation_author"].clone();
                    topic["creation_date"] = held["creation_date"].clone();
                    topic["modified_author"] = USER.into();
                    topic["modified_date"] = NOW.into();
                    *held = topic.clone();
                    (200, topic)
                }
                ("GET", [guid, "comments"]) => (
                    200,
                    Value::Array(state.comments.get(*guid).cloned().unwrap_or_default()),
                ),
                ("POST", [guid, "comments"]) => {
                    let mut comment = body();
                    comment["author"] = USER.into();
                    comment["date"] = NOW.into();
                    comment["topic_guid"] = (*guid).into();
                    state
                        .comments
                        .entry((*guid).to_owned())
                        .or_default()
                        .push(comment.clone());
                    (201, comment)
                }
                ("GET", [guid, "viewpoints"]) => (
                    200,
                    Value::Array(state.viewpoints.get(*guid).cloned().unwrap_or_default()),
                ),
                ("POST", [guid, "viewpoints"]) => {
                    let viewpoint = body();
                    state
                        .viewpoints
                        .entry((*guid).to_owned())
                        .or_default()
                        .push(viewpoint.clone());
                    (201, viewpoint)
                }
                _ => (404, json!({})),
            }
        }
        _ => (404, json!({})),
    }
}
