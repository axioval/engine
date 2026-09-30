#![allow(clippy::doc_markdown)]

//! Exchange of Axioval findings and review decisions with BCF API 3.0
//! servers (buildingSMART BCF REST API).
//!
//! The topics are the ones [`axioval_bcf::export`] writes to a file, and
//! what a server says about them is read back by the same mapping as a file
//! ([`axioval_bcf::import_topics`]), so a finding has one topic GUID, its
//! identity, whichever way it travels.
//!
//! - [`Client::connect`] checks the server speaks BCF API 3.0 and signs in
//!   ([`Auth`]): a bearer token the host obtained, or OAuth2 client
//!   credentials or the device flow.
//! - [`Client::push`] creates a report's topics, and updates the ones the
//!   server already has instead of duplicating them. Review state the
//!   report did not decide (status, priority, assignee, due date, a
//!   reviewer's labels) is kept as the server has it.
//! - [`Client::pull`] reads the project's topics with their comments as
//!   [`openbim_bcf::Markup`]s for [`axioval_bcf::import_topics`].
//!
//! Credentials live in [`Auth`] and the [`Client`] only: nothing here writes
//! them anywhere, and neither is `Debug`-printed with its secrets.
//!
//! # Transport
//!
//! Plain HTTP by default. HTTPS needs the `native-tls` feature, which uses
//! the platform's TLS library and certificate store; without it an `https`
//! server is refused with [`ApiError::TlsUnavailable`] rather than
//! contacted in the clear.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Write as _};
use std::time::{Duration, Instant};

use axioval_bcf::{ExportError, Options, export};
use axioval_ir::{FindingDecision, Project, Report};
use openbim_bcf::write::{self, Camera, Projection};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The BCF API version this client speaks.
pub const API_VERSION: &str = "3.0";

/// The OAuth2 grant type of the device flow (RFC 8628).
pub const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// How long one request may take.
const TIMEOUT: Duration = Duration::from_secs(60);

/// How the client signs in.
pub enum Auth {
    /// No authentication.
    None,
    /// A bearer token the host obtained itself.
    Bearer(String),
    /// OAuth2 client credentials, exchanged at the server's
    /// `oauth2_token_url` (or `token_url`, when given).
    ClientCredentials {
        /// The client's id.
        client_id: String,
        /// The client's secret.
        client_secret: String,
        /// The token endpoint, when the server's `/auth` names none.
        token_url: Option<String>,
    },
    /// The OAuth2 device flow (RFC 8628): a user code is shown to a person
    /// through `show`, who signs in elsewhere while the client polls the
    /// token endpoint. BCF API 3.0 does not publish a device authorization
    /// endpoint, so the host names it.
    Device {
        /// The client's id.
        client_id: String,
        /// Where a device code is requested.
        device_authorization_url: String,
        /// The token endpoint, when the server's `/auth` names none.
        token_url: Option<String>,
        /// Shows the person where to sign in and the code to enter.
        show: Box<dyn Fn(&DeviceCode)>,
    },
}

impl fmt::Debug for Auth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never print a secret.
        match self {
            Self::None => f.write_str("Auth::None"),
            Self::Bearer(_) => f.write_str("Auth::Bearer(..)"),
            Self::ClientCredentials { client_id, .. } => f
                .debug_struct("Auth::ClientCredentials")
                .field("client_id", client_id)
                .finish_non_exhaustive(),
            Self::Device { client_id, .. } => f
                .debug_struct("Auth::Device")
                .field("client_id", client_id)
                .finish_non_exhaustive(),
        }
    }
}

/// What a person needs to sign a device in.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct DeviceCode {
    /// The code to enter.
    pub user_code: String,
    /// Where to enter it.
    pub verification_uri: String,
    /// Where to go with the code already filled in, when the server offers
    /// one.
    #[serde(default)]
    pub verification_uri_complete: Option<String>,
    #[serde(rename = "device_code")]
    secret: String,
    #[serde(default = "default_expiry")]
    expires_in: u64,
    #[serde(default = "default_interval")]
    interval: u64,
}

fn default_expiry() -> u64 {
    600
}

fn default_interval() -> u64 {
    5
}

/// Why an exchange failed.
#[derive(Debug, Error)]
#[allow(missing_docs)] // Variant fields say what they hold by name.
pub enum ApiError {
    /// The server could not be reached, or the connection failed.
    #[error("{url}: {detail}")]
    Transport { url: String, detail: String },
    /// The server answered a request with an error status.
    #[error("{method} {url} answered {status}: {body}")]
    Status {
        method: &'static str,
        url: String,
        status: u16,
        body: String,
    },
    /// The server answered with a body this client cannot read.
    #[error("{url}: unexpected answer: {detail}")]
    Answer { url: String, detail: String },
    /// The server does not speak BCF API 3.0.
    #[error("{0} does not offer BCF API 3.0")]
    Unsupported(String),
    /// Signing in failed.
    #[error("signing in failed: {0}")]
    Auth(String),
    /// An `https` server was named, but this build has no TLS.
    #[error("{0}: HTTPS needs the `native-tls` feature of axioval-bcf-api")]
    TlsUnavailable(String),
    /// The report could not be mapped onto topics.
    #[error(transparent)]
    Export(#[from] ExportError),
}

/// A project on the server. Fields are named as in the BCF API 3.0 schema.
#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ProjectInfo {
    pub project_id: String,
    #[serde(default)]
    pub name: Option<String>,
}

/// A topic as the server holds it (fields this client reads), named as in
/// the BCF API 3.0 schema.
#[allow(missing_docs)]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiTopic {
    pub guid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_assigned_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic_status: Option<String>,
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assigned_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing)]
    pub creation_date: Option<String>,
    #[serde(default, skip_serializing)]
    pub creation_author: Option<String>,
    #[serde(default, skip_serializing)]
    pub modified_date: Option<String>,
    #[serde(default, skip_serializing)]
    pub modified_author: Option<String>,
}

/// A comment as the server holds it (fields this client reads), named as
/// in the BCF API 3.0 schema.
#[allow(missing_docs)]
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct ApiComment {
    pub guid: String,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub viewpoint_guid: Option<String>,
    #[serde(default)]
    pub modified_date: Option<String>,
    #[serde(default)]
    pub modified_author: Option<String>,
}

#[derive(Serialize)]
struct CommentRequest<'a> {
    guid: &'a str,
    comment: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    viewpoint_guid: Option<&'a str>,
}

#[derive(Deserialize)]
struct Guid {
    guid: String,
}

#[derive(Deserialize)]
struct Versions {
    versions: Vec<Version>,
}

#[derive(Deserialize)]
struct Version {
    version_id: String,
}

#[derive(Deserialize)]
struct AuthInfo {
    #[serde(default)]
    oauth2_token_url: Option<String>,
}

#[derive(Deserialize)]
struct Token {
    access_token: String,
}

#[derive(Deserialize)]
struct TokenError {
    error: String,
}

/// What a push did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pushed {
    /// Topics the server did not have.
    pub created: usize,
    /// Topics the server had and were updated.
    pub updated: usize,
    /// Comments added.
    pub comments: usize,
    /// Viewpoints added.
    pub viewpoints: usize,
}

/// A signed-in connection to one BCF API 3.0 server.
pub struct Client {
    agent: ureq::Agent,
    /// `…/bcf/3.0`.
    base: String,
    token: Option<String>,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The agent holds nothing worth printing, the token must not be.
        f.debug_struct("Client")
            .field("base", &self.base)
            .field("signed_in", &self.token.is_some())
            .finish_non_exhaustive()
    }
}

impl Client {
    /// Connects to the server at `server` (its root, before `/bcf`), checks
    /// it offers BCF API 3.0, and signs in with `auth`.
    ///
    /// # Errors
    ///
    /// [`ApiError::TlsUnavailable`] for an `https` server without TLS,
    /// [`ApiError::Unsupported`] when the server lists no 3.0, and
    /// [`ApiError::Auth`] when signing in fails.
    pub fn connect(server: &str, auth: &Auth) -> Result<Self, ApiError> {
        let server = server.trim_end_matches('/').to_owned();
        let secure = server
            .get(..8)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"));
        if secure && !cfg!(feature = "native-tls") {
            return Err(ApiError::TlsUnavailable(server));
        }
        let mut client = Self {
            agent: agent(),
            base: format!("{server}/bcf/{API_VERSION}"),
            token: None,
        };
        let url = format!("{server}/bcf/versions");
        let versions: Versions = client.get(&url)?.ok_or_else(|| missing(&url))?;
        if !versions
            .versions
            .iter()
            .any(|v| v.version_id == API_VERSION)
        {
            return Err(ApiError::Unsupported(server));
        }
        client.token = match auth {
            Auth::None => None,
            Auth::Bearer(token) => Some(token.clone()),
            Auth::ClientCredentials {
                client_id,
                client_secret,
                token_url,
            } => {
                let token_url = client.token_url(token_url.as_deref())?;
                Some(client.request_token(
                    &token_url,
                    &[
                        ("grant_type", "client_credentials"),
                        ("client_id", client_id),
                        ("client_secret", client_secret),
                    ],
                )?)
            }
            Auth::Device {
                client_id,
                device_authorization_url,
                token_url,
                show,
            } => {
                let token_url = client.token_url(token_url.as_deref())?;
                Some(client.device_token(client_id, device_authorization_url, &token_url, show)?)
            }
        };
        Ok(client)
    }

    /// The projects the signed-in user sees.
    ///
    /// # Errors
    ///
    /// As any request.
    pub fn projects(&self) -> Result<Vec<ProjectInfo>, ApiError> {
        let url = format!("{}/projects", self.base);
        self.get(&url)?.ok_or_else(|| missing(&url))
    }

    /// Every topic of `project`.
    ///
    /// # Errors
    ///
    /// As any request.
    pub fn topics(&self, project: &str) -> Result<Vec<ApiTopic>, ApiError> {
        let url = self.topics_url(project);
        self.get(&url)?.ok_or_else(|| missing(&url))
    }

    /// The topic `guid` of `project`, `None` when the server has none.
    ///
    /// # Errors
    ///
    /// As any request.
    pub fn topic(&self, project: &str, guid: &str) -> Result<Option<ApiTopic>, ApiError> {
        self.get(&format!("{}/{}", self.topics_url(project), segment(guid)))
    }

    /// The comments of topic `guid`, as the server orders them.
    ///
    /// # Errors
    ///
    /// As any request.
    pub fn comments(&self, project: &str, guid: &str) -> Result<Vec<ApiComment>, ApiError> {
        let url = format!("{}/{}/comments", self.topics_url(project), segment(guid));
        self.get(&url)?.ok_or_else(|| missing(&url))
    }

    /// Creates `report`'s topics in `project`, as [`export`] maps them with
    /// `options`, and updates those the server already has.
    ///
    /// A topic the server has is updated in place under its GUID, never
    /// duplicated. For a finding the report carries no decision about, the
    /// server's status, priority, assignee, due date and stage are kept and
    /// its labels are kept beside the report's: a push never overwrites
    /// review state it did not decide. A decided finding's topic takes the
    /// decision's, its assignee and due date included. Comments and
    /// viewpoints the server lacks are added; those it has are left alone,
    /// since the API makes viewpoints immutable.
    ///
    /// # Errors
    ///
    /// [`ApiError::Export`] when the report cannot be mapped onto topics,
    /// and as any request; topics pushed before the failure stay pushed.
    pub fn push(
        &self,
        project_id: &str,
        report: &Report,
        project: &Project,
        options: &Options,
    ) -> Result<Pushed, ApiError> {
        let exported = export(report, project, options)?;
        let decided: BTreeMap<String, &FindingDecision> = report
            .findings()
            .iter()
            .filter_map(|finding| Some((finding.id?.to_string(), finding.decision.as_ref()?)))
            .collect();
        let mut pushed = Pushed::default();
        for topic in &exported.document.topics {
            let decision = decided.get(&topic.guid).copied();
            let mut request = topic_request(topic, decision);
            let url = format!("{}/{}", self.topics_url(project_id), segment(&topic.guid));
            match self.topic(project_id, &topic.guid)? {
                None => {
                    let _: Guid = self.send("POST", &self.topics_url(project_id), &request)?;
                    pushed.created += 1;
                }
                Some(held) => {
                    if decision.is_none() {
                        keep_review(&mut request, held);
                    }
                    let _: Guid = self.send("PUT", &url, &request)?;
                    pushed.updated += 1;
                }
            }
            let viewpoints_url = format!("{url}/viewpoints");
            let held: BTreeSet<String> = self
                .get::<Vec<Guid>>(&viewpoints_url)?
                .unwrap_or_default()
                .into_iter()
                .map(|viewpoint| viewpoint.guid.to_ascii_lowercase())
                .collect();
            for viewpoint in &topic.viewpoints {
                if !held.contains(&viewpoint.guid.to_ascii_lowercase()) {
                    let _: Guid = self.send("POST", &viewpoints_url, &viewpoint_json(viewpoint))?;
                    pushed.viewpoints += 1;
                }
            }
            let comments_url = format!("{url}/comments");
            let held: BTreeSet<String> = self
                .comments(project_id, &topic.guid)?
                .into_iter()
                .map(|comment| comment.guid.to_ascii_lowercase())
                .collect();
            for comment in &topic.comments {
                if !held.contains(&comment.guid.to_ascii_lowercase()) {
                    let request = CommentRequest {
                        guid: &comment.guid,
                        comment: &comment.comment,
                        viewpoint_guid: comment.viewpoint.as_deref(),
                    };
                    let _: Guid = self.send("POST", &comments_url, &request)?;
                    pushed.comments += 1;
                }
            }
        }
        Ok(pushed)
    }

    /// Every topic of `project` with its comments, as BCF markups for
    /// [`axioval_bcf::import_topics`]. Viewpoints are not read: no decision
    /// is taken from them.
    ///
    /// # Errors
    ///
    /// As any request.
    pub fn pull(&self, project: &str) -> Result<Vec<openbim_bcf::Markup>, ApiError> {
        let mut markups = Vec::new();
        for topic in self.topics(project)? {
            let comments = self
                .comments(project, &topic.guid)?
                .into_iter()
                .map(|comment| openbim_bcf::Comment {
                    guid: Some(comment.guid),
                    date: comment.date,
                    author: comment.author,
                    comment: comment.comment,
                    viewpoint: comment.viewpoint_guid,
                    modified_date: comment.modified_date,
                    modified_author: comment.modified_author,
                })
                .collect();
            markups.push(openbim_bcf::Markup {
                entry: format!("{}/{}", self.topics_url(project), topic.guid),
                header_files: Vec::new(),
                topic: openbim_bcf::Topic {
                    guid: Some(topic.guid),
                    topic_type: topic.topic_type,
                    topic_status: topic.topic_status,
                    server_assigned_id: topic.server_assigned_id,
                    title: Some(topic.title),
                    priority: topic.priority,
                    index: None,
                    labels: topic.labels,
                    creation_date: topic.creation_date,
                    creation_author: topic.creation_author,
                    modified_date: topic.modified_date,
                    modified_author: topic.modified_author,
                    due_date: topic.due_date,
                    assigned_to: topic.assigned_to,
                    description: topic.description,
                    stage: topic.stage,
                    related_topics: Vec::new(),
                    reference_links: Vec::new(),
                },
                comments,
                viewpoints: Vec::new(),
            });
        }
        Ok(markups)
    }

    fn topics_url(&self, project: &str) -> String {
        format!("{}/projects/{}/topics", self.base, segment(project))
    }

    /// The token endpoint: `given`, or the server's `oauth2_token_url`.
    fn token_url(&self, given: Option<&str>) -> Result<String, ApiError> {
        if let Some(given) = given {
            return Ok(given.to_owned());
        }
        let url = format!("{}/auth", self.base);
        let info: AuthInfo = self.get(&url)?.ok_or_else(|| missing(&url))?;
        info.oauth2_token_url
            .ok_or_else(|| ApiError::Auth(format!("{url} names no oauth2_token_url")))
    }

    fn request_token(&self, url: &str, form: &[(&str, &str)]) -> Result<String, ApiError> {
        match self.form(url, form)? {
            Ok(token) => Ok(token),
            Err(error) => Err(ApiError::Auth(format!("{url}: {error}"))),
        }
    }

    /// Posts a token request: the access token, or the OAuth2 error code.
    fn form(&self, url: &str, form: &[(&str, &str)]) -> Result<Result<String, String>, ApiError> {
        let mut response = self
            .agent
            .post(url)
            .send_form(form.iter().copied())
            .map_err(|error| transport(url, &error))?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|error| transport(url, &error))?;
        if (200..300).contains(&status) {
            let token: Token = serde_json::from_str(&body).map_err(|error| answer(url, &error))?;
            return Ok(Ok(token.access_token));
        }
        Ok(Err(serde_json::from_str::<TokenError>(&body).map_or_else(
            |_| format!("status {status}"),
            |error| error.error,
        )))
    }

    fn device_token(
        &self,
        client_id: &str,
        device_url: &str,
        token_url: &str,
        show: &dyn Fn(&DeviceCode),
    ) -> Result<String, ApiError> {
        let mut response = self
            .agent
            .post(device_url)
            .send_form([("client_id", client_id)])
            .map_err(|error| transport(device_url, &error))?;
        if !response.status().is_success() {
            return Err(ApiError::Auth(format!(
                "{device_url} answered {}",
                response.status().as_u16()
            )));
        }
        let code: DeviceCode = response
            .body_mut()
            .read_json()
            .map_err(|error| answer(device_url, &error))?;
        show(&code);
        let deadline = Instant::now() + Duration::from_secs(code.expires_in);
        let mut interval = code.interval;
        loop {
            match self.form(
                token_url,
                &[
                    ("grant_type", DEVICE_CODE_GRANT),
                    ("device_code", &code.secret),
                    ("client_id", client_id),
                ],
            )? {
                Ok(token) => return Ok(token),
                Err(error) if error == "authorization_pending" => {}
                Err(error) if error == "slow_down" => interval += 5,
                Err(error) => return Err(ApiError::Auth(format!("{token_url}: {error}"))),
            }
            if Instant::now() >= deadline {
                return Err(ApiError::Auth("the device code expired".to_owned()));
            }
            std::thread::sleep(Duration::from_secs(interval));
        }
    }

    /// GETs `url` as JSON; `None` when the server has nothing there (404).
    fn get<T: DeserializeOwned>(&self, url: &str) -> Result<Option<T>, ApiError> {
        let mut request = self.agent.get(url);
        if let Some(token) = &self.token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        let mut response = request.call().map_err(|error| transport(url, &error))?;
        let status = response.status().as_u16();
        if status == 404 {
            return Ok(None);
        }
        if !(200..300).contains(&status) {
            let body = response.body_mut().read_to_string().unwrap_or_default();
            return Err(ApiError::Status {
                method: "GET",
                url: url.to_owned(),
                status,
                body,
            });
        }
        response
            .body_mut()
            .read_json()
            .map(Some)
            .map_err(|error| answer(url, &error))
    }

    /// POSTs or PUTs `body` as JSON to `url`.
    fn send<B: Serialize, T: DeserializeOwned>(
        &self,
        method: &'static str,
        url: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        let mut request = if method == "PUT" {
            self.agent.put(url)
        } else {
            self.agent.post(url)
        };
        if let Some(token) = &self.token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        let mut response = request
            .send_json(body)
            .map_err(|error| transport(url, &error))?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            let body = response.body_mut().read_to_string().unwrap_or_default();
            return Err(ApiError::Status {
                method,
                url: url.to_owned(),
                status,
                body,
            });
        }
        response
            .body_mut()
            .read_json()
            .map_err(|error| answer(url, &error))
    }
}

/// An agent that returns error statuses as answers, never follows a
/// redirect with the token to another host, and uses the platform's TLS
/// when built with it.
fn agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(TIMEOUT))
        .user_agent(concat!("axioval-bcf-api/", env!("CARGO_PKG_VERSION")));
    #[cfg(feature = "native-tls")]
    let config = config.tls_config(
        ureq::tls::TlsConfig::builder()
            .provider(ureq::tls::TlsProvider::NativeTls)
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .build(),
    );
    config.build().into()
}

fn transport(url: &str, error: &ureq::Error) -> ApiError {
    ApiError::Transport {
        url: url.to_owned(),
        detail: error.to_string(),
    }
}

fn answer(url: &str, error: &dyn fmt::Display) -> ApiError {
    ApiError::Answer {
        url: url.to_owned(),
        detail: error.to_string(),
    }
}

fn missing(url: &str) -> ApiError {
    ApiError::Answer {
        url: url.to_owned(),
        detail: "not found".to_owned(),
    }
}

/// `text` as one URL path segment: unreserved characters kept, every other
/// byte percent-encoded.
fn segment(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

/// The topic the export wrote, with the decision's assignee and due date,
/// which the BCF API carries although a BCF file cannot yet.
fn topic_request(topic: &write::Topic, decision: Option<&FindingDecision>) -> ApiTopic {
    ApiTopic {
        guid: topic.guid.clone(),
        topic_type: topic.topic_type.clone(),
        topic_status: topic.topic_status.clone(),
        title: topic.title.clone(),
        priority: topic.priority.clone(),
        labels: topic.labels.clone(),
        assigned_to: decision.and_then(|d| d.assigned_to.clone()),
        due_date: decision
            .and_then(|d| d.due_date)
            .map(|date| date.to_string()),
        description: topic.description.clone(),
        ..ApiTopic::default()
    }
}

/// Keeps the review state the server holds for an undecided finding.
fn keep_review(request: &mut ApiTopic, held: ApiTopic) {
    request.topic_status = held.topic_status.or(request.topic_status.take());
    request.priority = held.priority.or(request.priority.take());
    request.assigned_to = held.assigned_to;
    request.due_date = held.due_date;
    request.stage = held.stage;
    for label in held.labels {
        if !request.labels.contains(&label) {
            request.labels.push(label);
        }
    }
}

/// A viewpoint as BCF API 3.0 JSON. The API requires an aspect ratio on
/// every camera; a 2.1-shaped camera gets the export's
/// [`axioval_bcf::ASPECT_RATIO`].
fn viewpoint_json(viewpoint: &write::Viewpoint) -> serde_json::Value {
    use serde_json::{Value, json};
    let point = |v: write::Vector3| json!({"x": v.x, "y": v.y, "z": v.z});
    let component = |c: &openbim_bcf::Component| {
        let mut value = serde_json::Map::new();
        if let Some(guid) = &c.ifc_guid {
            value.insert("ifc_guid".into(), guid.clone().into());
        }
        if let Some(system) = &c.originating_system {
            value.insert("originating_system".into(), system.clone().into());
        }
        if let Some(id) = &c.authoring_tool_id {
            value.insert("authoring_tool_id".into(), id.clone().into());
        }
        Value::Object(value)
    };
    let components =
        |list: &[openbim_bcf::Component]| -> Vec<Value> { list.iter().map(component).collect() };
    let mut json = serde_json::Map::new();
    json.insert("guid".into(), viewpoint.guid.clone().into());
    if let Some(Camera {
        projection,
        view_point,
        direction,
        up_vector,
        aspect_ratio,
    }) = viewpoint.camera
    {
        let mut camera = json!({
            "camera_view_point": point(view_point),
            "camera_direction": point(direction),
            "camera_up_vector": point(up_vector),
            "aspect_ratio": aspect_ratio.unwrap_or(axioval_bcf::ASPECT_RATIO),
        });
        match projection {
            Projection::Perspective { field_of_view } => {
                camera["field_of_view"] = field_of_view.into();
                json.insert("perspective_camera".into(), camera);
            }
            Projection::Orthogonal {
                view_to_world_scale,
            } => {
                camera["view_to_world_scale"] = view_to_world_scale.into();
                json.insert("orthogonal_camera".into(), camera);
            }
        }
    }
    if !viewpoint.clipping_planes.is_empty() {
        let planes: Vec<Value> = viewpoint
            .clipping_planes
            .iter()
            .map(|plane| json!({"location": point(plane.location), "direction": point(plane.direction)}))
            .collect();
        json.insert("clipping_planes".into(), planes.into());
    }
    let visibility = viewpoint.visibility.as_ref().map_or_else(
        || json!({"default_visibility": true}),
        |visibility| {
            json!({
                "default_visibility": visibility.default_visibility,
                "exceptions": components(&visibility.exceptions),
            })
        },
    );
    let coloring: Vec<Value> = viewpoint
        .coloring
        .iter()
        .map(|coloring| json!({"color": coloring.color, "components": components(&coloring.components)}))
        .collect();
    let mut parts = json!({
        "selection": components(&viewpoint.selection),
        "visibility": visibility,
    });
    if !coloring.is_empty() {
        parts["coloring"] = coloring.into();
    }
    json.insert("components".into(), parts);
    Value::Object(json)
}

#[cfg(test)]
mod tests {
    use super::segment;

    #[test]
    fn path_segments_are_percent_encoded() {
        assert_eq!(segment("a b/c%d"), "a%20b%2Fc%25d");
        assert_eq!(segment("3f2504e0-4f89"), "3f2504e0-4f89");
    }
}
