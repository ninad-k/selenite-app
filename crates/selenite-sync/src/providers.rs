//! Provider HTTP adapters. The remote folder contains immutable JSON version
//! objects; each embeds exact source bytes and a portable logical vault path.

use crate::{
    MAX_REMOTE_VERSIONS, MAX_VERSION_BYTES, ProviderKind, RemoteVersion, SyncConfig, SyncProvider,
    digest, endpoint, valid_hash,
};
use anyhow::{Context, Result, bail, ensure};
use reqwest::{
    StatusCode,
    blocking::{Client, RequestBuilder, Response},
    header::LOCATION,
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, io::Read, time::Duration};
use url::Url;

pub fn configured_provider(config: &SyncConfig) -> Result<Box<dyn SyncProvider>> {
    ensure!(config.enabled, "sync is disabled in sync.toml");
    config.validate()?;
    match config.provider {
        ProviderKind::GoogleDrive => Ok(Box::new(GoogleDrive::new(config.clone())?)),
        ProviderKind::OneDrive => Ok(Box::new(OneDrive::new(config.clone())?)),
    }
}

struct Http {
    client: Client,
    token: String,
}

impl Http {
    fn new(config: &SyncConfig) -> Result<Self> {
        config.validate()?;
        let token = std::env::var(&config.token_env).with_context(|| {
            format!(
                "set the environment variable named by token_env ({})",
                config.token_env
            )
        })?;
        ensure!(
            !token.trim().is_empty() && !token.contains(['\r', '\n']),
            "sync access token is empty or invalid"
        );
        Ok(Self {
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            token,
        })
    }

    fn send(&self, request: RequestBuilder) -> Result<Response> {
        // Do not expose URLs with signed query credentials in transport errors.
        request
            .send()
            .map_err(|_| anyhow::anyhow!("sync transport request failed"))
    }

    fn get_json(&self, url: Url) -> Result<Value> {
        parse_json(self.send(self.client.get(url).bearer_auth(&self.token))?)
    }

    fn content(&self, url: Url, redirects: bool) -> Result<Vec<u8>> {
        let mut response = self.send(self.client.get(url).bearer_auth(&self.token))?;
        for _ in 0..3 {
            if !response.status().is_redirection() {
                return read_response(response);
            }
            ensure!(redirects, "unexpected redirect from sync provider");
            let location = response
                .headers()
                .get(LOCATION)
                .context("download redirect has no location")?
                .to_str()?;
            let url = Url::parse(location).context("invalid download redirect")?;
            validate_transfer_url(&url)?;
            // Graph download URLs are preauthenticated. Never forward the bearer.
            response = self.send(self.client.get(url))?;
        }
        bail!("too many sync download redirects")
    }
}

fn status(response: &Response) -> Result<()> {
    match response.status() {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => bail!(
            "sync authorization failed (HTTP {})",
            response.status().as_u16()
        ),
        StatusCode::NOT_FOUND => bail!("remote folder or version is missing (HTTP 404)"),
        code if !code.is_success() => bail!("sync provider returned HTTP {}", code.as_u16()),
        _ => Ok(()),
    }
}

fn read_response(response: Response) -> Result<Vec<u8>> {
    status(&response)?;
    ensure!(
        response
            .content_length()
            .is_none_or(|length| length <= MAX_VERSION_BYTES as u64),
        "sync response exceeds size limit"
    );
    let mut bytes = Vec::new();
    response
        .take((MAX_VERSION_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .context("cannot read sync response")?;
    ensure!(
        bytes.len() <= MAX_VERSION_BYTES,
        "sync response exceeds size limit"
    );
    Ok(bytes)
}

fn parse_json(response: Response) -> Result<Value> {
    serde_json::from_slice(&read_response(response)?).context("invalid provider JSON")
}

fn joined(base: &Url, parts: &[&str]) -> Result<Url> {
    ensure!(
        parts.iter().all(|part| !matches!(*part, "" | "." | "..")),
        "invalid provider URL path component"
    );
    let mut url = base.clone();
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| anyhow::anyhow!("invalid API base URL"))?;
        segments.pop_if_empty();
        for part in parts {
            segments.push(part);
        }
    }
    Ok(url)
}

fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .with_context(|| format!("provider response lacks {key}"))
}
fn archive_name(revision: &str) -> String {
    format!("{revision}.selenite-version.json")
}
fn revision_from_name(name: &str) -> Option<&str> {
    name.strip_suffix(".selenite-version.json")
        .filter(|revision| valid_hash(revision))
}
fn decode_version(bytes: &[u8], revision: &str) -> Result<RemoteVersion> {
    let version: RemoteVersion =
        serde_json::from_slice(bytes).context("invalid immutable version object")?;
    version.validate()?;
    ensure!(
        version.revision == revision,
        "remote version object was modified or renamed"
    );
    Ok(version)
}

fn validate_transfer_url(url: &Url) -> Result<()> {
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    ensure!(
        (url.scheme() == "https" || (url.scheme() == "http" && loopback))
            && url.username().is_empty()
            && url.password().is_none(),
        "unsafe preauthenticated transfer URL"
    );
    Ok(())
}

fn identity(config: &SyncConfig, provider: &str) -> String {
    format!(
        "{provider}:{}",
        digest(
            serde_json::to_vec(&(
                config.endpoint.as_str(),
                &config.upload_endpoint,
                config.folder_id.as_str(),
                &config.drive_id,
                config.token_env.as_str()
            ))
            .expect("string configuration serializes")
        )
    )
}

pub struct GoogleDrive {
    config: SyncConfig,
    http: Http,
    base: Url,
    upload: Url,
}

impl GoogleDrive {
    pub fn new(config: SyncConfig) -> Result<Self> {
        let http = Http::new(&config)?;
        let base = endpoint(&config.endpoint)?;
        let upload = if let Some(upload) = &config.upload_endpoint {
            endpoint(upload)?
        } else {
            let mut upload = base.clone();
            upload.set_path(&format!("/upload{}", base.path()));
            upload
        };
        Ok(Self {
            config,
            http,
            base,
            upload,
        })
    }
}

impl SyncProvider for GoogleDrive {
    fn name(&self) -> &str {
        "Google Drive"
    }
    fn identity(&self) -> String {
        identity(&self.config, "google_drive")
    }
    fn versions(&self) -> Result<Vec<RemoteVersion>> {
        let mut versions = Vec::new();
        let mut page: Option<String> = None;
        let mut seen = BTreeSet::new();
        loop {
            let mut url = joined(&self.base, &["files"])?;
            {
                let mut query = url.query_pairs_mut();
                query.append_pair("q", &format!("'{}' in parents and trashed = false and appProperties has {{ key='seleniteFormat' and value='1' }}", self.config.folder_id));
                query
                    .append_pair("pageSize", "1000")
                    .append_pair(
                        "fields",
                        "files(id,name,parents,appProperties),nextPageToken,incompleteSearch",
                    )
                    .append_pair("supportsAllDrives", "true")
                    .append_pair("includeItemsFromAllDrives", "true");
                if let Some(drive) = &self.config.drive_id {
                    query
                        .append_pair("corpora", "drive")
                        .append_pair("driveId", drive);
                }
                if let Some(page) = &page {
                    query.append_pair("pageToken", page);
                }
            }
            let response = self.http.get_json(url)?;
            ensure!(
                response["incompleteSearch"].as_bool() != Some(true),
                "Google Drive listing is incomplete; sync stopped"
            );
            for file in response["files"]
                .as_array()
                .context("Drive listing has no files array")?
            {
                let name = string(file, "name")?;
                let Some(revision) = revision_from_name(name) else {
                    continue;
                };
                ensure!(
                    file["parents"].as_array().is_some_and(|parents| parents
                        .iter()
                        .any(|parent| parent.as_str() == Some(&self.config.folder_id))),
                    "Drive returned a version outside the configured folder"
                );
                ensure!(
                    file["appProperties"]["seleniteFormat"].as_str() == Some("1"),
                    "invalid Drive archive marker"
                );
                ensure!(
                    versions.len() < MAX_REMOTE_VERSIONS,
                    "remote archive is too large"
                );
                let mut url = joined(&self.base, &["files", string(file, "id")?])?;
                url.query_pairs_mut()
                    .append_pair("alt", "media")
                    .append_pair("supportsAllDrives", "true");
                versions.push(decode_version(&self.http.content(url, false)?, revision)?);
            }
            match response["nextPageToken"]
                .as_str()
                .filter(|token| !token.is_empty())
            {
                Some(token) => {
                    ensure!(seen.insert(token.to_owned()), "Drive pagination loop");
                    page = Some(token.to_owned());
                }
                None => break,
            }
        }
        Ok(versions)
    }
    fn publish(&self, version: &RemoteVersion) -> Result<()> {
        version.validate()?;
        let bytes = serde_json::to_string(version)?;
        ensure!(
            bytes.len() <= MAX_VERSION_BYTES,
            "version object exceeds upload limit"
        );
        let metadata = json!({"name":archive_name(&version.revision),"parents":[self.config.folder_id],"mimeType":"application/json","appProperties":{"seleniteFormat":"1","selenitePath":digest(&version.path)}});
        let boundary = format!("selenite_{}", version.revision);
        let body = format!(
            "--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{metadata}\r\n--{boundary}\r\nContent-Type: application/json\r\n\r\n{bytes}\r\n--{boundary}--\r\n"
        );
        let mut url = joined(&self.upload, &["files"])?;
        url.query_pairs_mut()
            .append_pair("uploadType", "multipart")
            .append_pair("supportsAllDrives", "true")
            .append_pair("fields", "id");
        let response = self.http.send(
            self.http
                .client
                .post(url)
                .bearer_auth(&self.http.token)
                .header(
                    "Content-Type",
                    format!("multipart/related; boundary={boundary}"),
                )
                .body(body),
        )?;
        let result = parse_json(response)?;
        string(&result, "id")?;
        Ok(())
    }
}

pub struct OneDrive {
    config: SyncConfig,
    http: Http,
    base: Url,
}

impl OneDrive {
    pub fn new(config: SyncConfig) -> Result<Self> {
        let http = Http::new(&config)?;
        let base = endpoint(&config.endpoint)?;
        Ok(Self { config, http, base })
    }

    fn drive_url(&self, rest: &[&str]) -> Result<Url> {
        let prefix = if let Some(drive) = &self.config.drive_id {
            vec!["drives", drive.as_str()]
        } else {
            vec!["me", "drive"]
        };
        let parts: Vec<_> = prefix.into_iter().chain(rest.iter().copied()).collect();
        joined(&self.base, &parts)
    }

    fn existing_is_identical(&self, version: &RemoteVersion) -> Result<()> {
        let existing = self
            .versions()?
            .into_iter()
            .find(|candidate| candidate.revision == version.revision)
            .context("remote create raced with another object; nothing was overwritten")?;
        ensure!(
            &existing == version,
            "remote create conflict; existing bytes retained"
        );
        Ok(())
    }
}

impl SyncProvider for OneDrive {
    fn name(&self) -> &str {
        "OneDrive"
    }
    fn identity(&self) -> String {
        identity(&self.config, "one_drive")
    }
    fn versions(&self) -> Result<Vec<RemoteVersion>> {
        let first = self.drive_url(&["items", &self.config.folder_id, "children"])?;
        let mut url = first.clone();
        url.query_pairs_mut()
            .append_pair("$select", "id,name,file,folder,parentReference");
        let mut seen = BTreeSet::new();
        let mut versions = Vec::new();
        loop {
            ensure!(
                seen.insert(url.as_str().to_owned()),
                "Graph pagination loop"
            );
            let response = self.http.get_json(url)?;
            for file in response["value"]
                .as_array()
                .context("Graph listing has no value array")?
            {
                let Some(revision) = revision_from_name(string(file, "name")?) else {
                    continue;
                };
                ensure!(
                    file["file"].is_object()
                        && file["parentReference"]["id"].as_str() == Some(&self.config.folder_id),
                    "Graph returned a version outside the configured folder"
                );
                ensure!(
                    versions.len() < MAX_REMOTE_VERSIONS,
                    "remote archive is too large"
                );
                let url = self.drive_url(&["items", string(file, "id")?, "content"])?;
                versions.push(decode_version(&self.http.content(url, true)?, revision)?);
            }
            let Some(next) = response["@odata.nextLink"].as_str() else {
                break;
            };
            url = Url::parse(next).context("invalid Graph pagination URL")?;
            ensure!(
                url.origin() == first.origin()
                    && url.path() == first.path()
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.fragment().is_none(),
                "Graph pagination escaped configured endpoint/folder"
            );
        }
        Ok(versions)
    }
    fn publish(&self, version: &RemoteVersion) -> Result<()> {
        version.validate()?;
        let bytes = serde_json::to_vec(version)?;
        ensure!(
            !bytes.is_empty() && bytes.len() <= MAX_VERSION_BYTES,
            "version object exceeds upload limit"
        );
        let name = archive_name(&version.revision);
        let parent = format!("{}:", self.config.folder_id);
        let file = format!("{name}:");
        let url = self.drive_url(&["items", &parent, &file, "createUploadSession"])?;
        let request = self
            .http
            .client
            .post(url)
            .bearer_auth(&self.http.token)
            .json(&json!({"item":{"@microsoft.graph.conflictBehavior":"fail","name":name}}));
        let response = self.http.send(request)?;
        if response.status() == StatusCode::CONFLICT {
            return self.existing_is_identical(version);
        }
        let response = parse_json(response)?;
        let upload = Url::parse(string(&response, "uploadUrl")?).context("invalid upload URL")?;
        validate_transfer_url(&upload)?;
        let length = bytes.len();
        let response = self.http.send(
            self.http
                .client
                .put(upload)
                .header("Content-Type", "application/octet-stream")
                .header("Content-Range", format!("bytes 0-{}/{length}", length - 1))
                .body(bytes),
        )?;
        if response.status() == StatusCode::CONFLICT {
            return self.existing_is_identical(version);
        }
        ensure!(
            matches!(response.status(), StatusCode::OK | StatusCode::CREATED),
            "immutable Graph upload did not commit (HTTP {})",
            response.status().as_u16()
        );
        status(&response)
    }
}

#[cfg(test)]
mod tests;
