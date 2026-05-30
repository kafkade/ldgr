//! `WebDAV` blob transport implementation.
//!
//! Uses standard `WebDAV` HTTP methods (PUT, GET, PROPFIND, MKCOL, DELETE)
//! for file operations. Supports Basic and Digest authentication.
//! Tested with Nextcloud, ownCloud, and generic `WebDAV` servers.

use ldgr_core::sync::transport::{
    BlobEntry, BlobPath, BlobPrefix, ListResult, PutResult, TransportErrorKind,
};
use reqwest::Client;

use super::{BlobTransport, TransportError};

/// `WebDAV` transport configuration.
#[derive(Debug, Clone)]
pub struct WebDavTransport {
    client: Client,
    base_url: String,
    username: String,
    password: String,
}

impl WebDavTransport {
    /// Create a new `WebDAV` transport.
    ///
    /// `base_url` should be the full URL to the `WebDAV` directory,
    /// e.g., `https://cloud.example.com/remote.php/dav/files/user/ldgr/`
    pub fn new(base_url: String, username: String, password: String) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("failed to create HTTP client");

        // Ensure base URL ends with /
        let base_url = if base_url.ends_with('/') {
            base_url
        } else {
            format!("{base_url}/")
        };

        Self {
            client,
            base_url,
            username,
            password,
        }
    }

    /// Build the full URL for a blob path.
    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// Classify an HTTP status into a transport error kind.
    fn classify_status(status: reqwest::StatusCode) -> TransportErrorKind {
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            TransportErrorKind::Auth
        } else if status == reqwest::StatusCode::NOT_FOUND {
            TransportErrorKind::NotFound
        } else if status == reqwest::StatusCode::CONFLICT {
            TransportErrorKind::Conflict
        } else if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            TransportErrorKind::RateLimited
        } else if status.is_server_error() {
            TransportErrorKind::Server
        } else {
            TransportErrorKind::Other
        }
    }

    /// Parse a PROPFIND XML response into blob entries.
    fn parse_propfind(body: &str, prefix: &str) -> Vec<BlobEntry> {
        // Simple XML parsing — WebDAV responses use the DAV: namespace
        // We parse <d:response> elements to extract href, size, and dates.
        let mut entries = Vec::new();

        for response_block in body.split("<d:response>").skip(1) {
            let end = response_block
                .find("</d:response>")
                .unwrap_or(response_block.len());
            let block = &response_block[..end];

            // Extract href
            let href = extract_xml_value(block, "d:href").unwrap_or_default();

            // Skip the directory itself
            if href.ends_with('/') {
                continue;
            }

            // Extract content length
            let size: u64 = extract_xml_value(block, "d:getcontentlength")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);

            // Extract last modified
            let modified_at = extract_xml_value(block, "d:getlastmodified");

            // Extract etag
            let etag = extract_xml_value(block, "d:getetag");

            // Convert href to a relative path
            let path = href.strip_prefix('/').unwrap_or(&href).to_string();

            // Filter by prefix
            if !path.is_empty() && (prefix.is_empty() || path.contains(prefix)) {
                entries.push(BlobEntry {
                    path,
                    size,
                    content_hash: etag,
                    modified_at,
                });
            }
        }

        entries
    }
}

/// Extract a value from simple XML like `<tag>value</tag>`.
fn extract_xml_value(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].trim().to_string())
}

// ── BlobTransport Implementation ───────────────────────────────────────────────

#[async_trait::async_trait]
impl BlobTransport for WebDavTransport {
    async fn put_blob(&self, path: &BlobPath, data: &[u8]) -> Result<PutResult, TransportError> {
        let url = self.url(path.as_str());

        let resp = self
            .client
            .put(&url)
            .basic_auth(&self.username, Some(&self.password))
            .header("Content-Type", "application/octet-stream")
            // If-None-Match: * means only create if file doesn't exist
            .header("If-None-Match", "*")
            .body(data.to_vec())
            .send()
            .await
            .map_err(|e| {
                TransportError::with_source(TransportErrorKind::Network, "PUT failed", e)
            })?;

        let status = resp.status();

        // 412 Precondition Failed means file already exists (put-if-absent)
        if status == reqwest::StatusCode::PRECONDITION_FAILED {
            return Err(TransportError::new(
                TransportErrorKind::Conflict,
                "blob already exists",
            ));
        }

        // 201 Created or 204 No Content are success
        if status == reqwest::StatusCode::CREATED || status == reqwest::StatusCode::NO_CONTENT {
            let etag = resp
                .headers()
                .get("etag")
                .and_then(|v| v.to_str().ok())
                .map(String::from);

            return Ok(PutResult {
                content_hash: None,
                etag,
            });
        }

        let body = resp.text().await.unwrap_or_default();
        Err(TransportError::new(
            Self::classify_status(status),
            format!("PUT failed ({status}): {body}"),
        ))
    }

    async fn get_blob(&self, path: &BlobPath) -> Result<Vec<u8>, TransportError> {
        let url = self.url(path.as_str());

        let resp = self
            .client
            .get(&url)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await
            .map_err(|e| {
                TransportError::with_source(TransportErrorKind::Network, "GET failed", e)
            })?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(TransportError::new(
                Self::classify_status(status),
                format!("GET failed ({status}): {body}"),
            ));
        }

        resp.bytes().await.map(|b| b.to_vec()).map_err(|e| {
            TransportError::with_source(TransportErrorKind::Network, "failed to read response", e)
        })
    }

    async fn list_blobs(
        &self,
        prefix: &BlobPrefix,
        _cursor: Option<&str>,
    ) -> Result<ListResult, TransportError> {
        let url = self.url(prefix.as_str());

        // PROPFIND with Depth: infinity to list all files recursively
        let propfind_body = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:">
  <d:prop>
    <d:getcontentlength/>
    <d:getlastmodified/>
    <d:getetag/>
    <d:resourcetype/>
  </d:prop>
</d:propfind>"#;

        let resp = self
            .client
            .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), &url)
            .basic_auth(&self.username, Some(&self.password))
            .header("Content-Type", "application/xml")
            .header("Depth", "infinity")
            .body(propfind_body)
            .send()
            .await
            .map_err(|e| {
                TransportError::with_source(TransportErrorKind::Network, "PROPFIND failed", e)
            })?;

        let status = resp.status();
        // 207 Multi-Status is the success response for PROPFIND
        if status.as_u16() != 207 && !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(TransportError::new(
                Self::classify_status(status),
                format!("PROPFIND failed ({status}): {body}"),
            ));
        }

        let body = resp.text().await.map_err(|e| {
            TransportError::with_source(
                TransportErrorKind::Network,
                "failed to read PROPFIND response",
                e,
            )
        })?;

        let entries = Self::parse_propfind(&body, prefix.as_str());

        Ok(ListResult {
            entries,
            cursor: None, // WebDAV doesn't have cursors
            has_more: false,
        })
    }

    async fn delete_blob(&self, path: &BlobPath) -> Result<(), TransportError> {
        let url = self.url(path.as_str());

        let resp = self
            .client
            .delete(&url)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await
            .map_err(|e| {
                TransportError::with_source(TransportErrorKind::Network, "DELETE failed", e)
            })?;

        let status = resp.status();
        // 204 No Content or 404 Not Found are both ok (idempotent delete)
        if status == reqwest::StatusCode::NO_CONTENT
            || status == reqwest::StatusCode::OK
            || status == reqwest::StatusCode::NOT_FOUND
        {
            return Ok(());
        }

        let body = resp.text().await.unwrap_or_default();
        Err(TransportError::new(
            Self::classify_status(status),
            format!("DELETE failed ({status}): {body}"),
        ))
    }

    async fn exists(&self, path: &BlobPath) -> Result<bool, TransportError> {
        let url = self.url(path.as_str());

        let resp = self
            .client
            .head(&url)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await
            .map_err(|e| {
                TransportError::with_source(TransportErrorKind::Network, "HEAD failed", e)
            })?;

        let status = resp.status();
        if status.is_success() {
            Ok(true)
        } else if status == reqwest::StatusCode::NOT_FOUND {
            Ok(false)
        } else {
            Err(TransportError::new(
                Self::classify_status(status),
                format!("HEAD failed ({status})"),
            ))
        }
    }

    async fn ensure_directory(&self, prefix: &BlobPrefix) -> Result<(), TransportError> {
        // Create directories recursively by trying each path segment
        let path = prefix.as_str().trim_end_matches('/');
        let segments: Vec<&str> = path.split('/').collect();

        let mut current = String::new();
        for segment in segments {
            if !current.is_empty() {
                current.push('/');
            }
            current.push_str(segment);

            let url = self.url(&format!("{current}/"));

            let resp = self
                .client
                .request(reqwest::Method::from_bytes(b"MKCOL").unwrap(), &url)
                .basic_auth(&self.username, Some(&self.password))
                .send()
                .await
                .map_err(|e| {
                    TransportError::with_source(TransportErrorKind::Network, "MKCOL failed", e)
                })?;

            let status = resp.status();
            // 201 Created, 405 Method Not Allowed (exists), or 409 Conflict (parent
            // missing — shouldn't happen with recursive creation) are acceptable
            if status == reqwest::StatusCode::CREATED
                || status == reqwest::StatusCode::METHOD_NOT_ALLOWED
            {
                continue;
            }

            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(TransportError::new(
                    Self::classify_status(status),
                    format!("MKCOL failed for {current} ({status}): {body}"),
                ));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_propfind_response() {
        let xml = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:">
  <d:response>
    <d:href>/remote.php/dav/files/user/ldgr/batches/</d:href>
    <d:propstat>
      <d:prop>
        <d:resourcetype><d:collection/></d:resourcetype>
      </d:prop>
    </d:propstat>
  </d:response>
  <d:response>
    <d:href>/remote.php/dav/files/user/ldgr/batches/dev1/batch_001.enc</d:href>
    <d:propstat>
      <d:prop>
        <d:getcontentlength>4096</d:getcontentlength>
        <d:getlastmodified>Mon, 15 Jan 2024 10:30:00 GMT</d:getlastmodified>
        <d:getetag>"abc123"</d:getetag>
      </d:prop>
    </d:propstat>
  </d:response>
  <d:response>
    <d:href>/remote.php/dav/files/user/ldgr/batches/dev1/batch_002.enc</d:href>
    <d:propstat>
      <d:prop>
        <d:getcontentlength>8192</d:getcontentlength>
        <d:getlastmodified>Tue, 16 Jan 2024 11:00:00 GMT</d:getlastmodified>
        <d:getetag>"def456"</d:getetag>
      </d:prop>
    </d:propstat>
  </d:response>
</d:multistatus>"#;

        let entries = WebDavTransport::parse_propfind(xml, "batches/");
        assert_eq!(entries.len(), 2);
        assert!(entries[0].path.contains("batch_001.enc"));
        assert_eq!(entries[0].size, 4096);
        assert_eq!(entries[0].content_hash.as_deref(), Some("\"abc123\""));
        assert_eq!(entries[1].size, 8192);
    }

    #[test]
    fn extract_xml_value_works() {
        assert_eq!(
            extract_xml_value(
                "<d:getcontentlength>4096</d:getcontentlength>",
                "d:getcontentlength"
            ),
            Some("4096".to_string())
        );
        assert_eq!(
            extract_xml_value("<d:getetag>\"abc\"</d:getetag>", "d:getetag"),
            Some("\"abc\"".to_string())
        );
        assert_eq!(extract_xml_value("<foo>bar</foo>", "baz"), None);
    }
}
