use anyhow::{bail, Context, Result};
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct MicroCmsClient {
    api_key: String,
    http: reqwest::Client,
    content_api_url: String,
    management_api_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiList {
    pub apis: Vec<ApiInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiInfo {
    pub endpoint: String,
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentCollectionKind {
    List,
    Object,
}

#[derive(Debug, Clone)]
pub struct ContentCollection {
    pub kind: ContentCollectionKind,
    pub total_count: usize,
    pub offset: usize,
    pub limit: usize,
    pub contents: Vec<Value>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ContentMetaList {
    pub contents: Vec<ContentMeta>,
    #[cfg_attr(not(test), allow(dead_code))]
    #[serde(rename = "totalCount")]
    pub total_count: usize,
    #[cfg_attr(not(test), allow(dead_code))]
    pub offset: usize,
    #[cfg_attr(not(test), allow(dead_code))]
    pub limit: usize,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ContentMeta {
    pub id: String,
    #[serde(default)]
    pub status: Vec<String>,
    #[serde(default, rename = "draftKey")]
    pub draft_key: Option<String>,
    #[serde(default, rename = "reservationTime")]
    pub reservation_time: Option<ReservationTime>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct ReservationTime {
    #[serde(default, rename = "publishTime")]
    pub publish_time: Option<String>,
    #[serde(default, rename = "stopTime")]
    pub stop_time: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContentQuery {
    pub q: Option<String>,
    pub filters: Option<String>,
    pub orders: Option<String>,
    pub fields: Option<String>,
    pub depth: Option<u8>,
    pub ids: Option<String>,
    pub draft_key: Option<String>,
    pub rich_editor_format: Option<String>,
}

impl ContentQuery {
    #[cfg(test)]
    pub fn has_query(&self) -> bool {
        [
            &self.q,
            &self.filters,
            &self.orders,
            &self.fields,
            &self.ids,
            &self.draft_key,
            &self.rich_editor_format,
        ]
        .into_iter()
        .any(|value| nonempty(value).is_some())
            || self.depth.is_some()
    }

    fn query_pairs(&self) -> Vec<(&'static str, String)> {
        let mut pairs = Vec::new();
        for (key, value) in [
            ("q", &self.q),
            ("filters", &self.filters),
            ("orders", &self.orders),
            ("fields", &self.fields),
            ("ids", &self.ids),
            ("draftKey", &self.draft_key),
            ("richEditorFormat", &self.rich_editor_format),
        ] {
            if let Some(value) = nonempty(value) {
                pairs.push((key, value.to_string()));
            }
        }
        if let Some(depth) = self.depth {
            pairs.push(("depth", depth.to_string()));
        }
        pairs
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationStatus {
    Publish,
    Draft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentWriteStatus {
    Default,
    Draft,
}

impl ContentWriteStatus {
    fn query_value(self) -> Option<&'static str> {
        match self {
            Self::Default => None,
            Self::Draft => Some("draft"),
        }
    }
}

impl PublicationStatus {
    fn api_value(self) -> &'static str {
        match self {
            Self::Publish => "PUBLISH",
            Self::Draft => "DRAFT",
        }
    }
}

impl MicroCmsClient {
    pub fn new(service_id: impl Into<String>, api_key: impl Into<String>) -> Result<Self> {
        let service_id = service_id.into();
        let api_key = api_key.into();
        if service_id.trim().is_empty() {
            bail!("service_id is missing or empty");
        }
        if api_key.trim().is_empty() {
            bail!("api_key is missing or empty");
        }

        let (content_api_url, management_api_url) = api_urls_from_env(&service_id)?;

        Ok(Self {
            api_key,
            http: reqwest::Client::new(),
            content_api_url,
            management_api_url,
        })
    }

    pub async fn list_apis(&self) -> Result<ApiList> {
        let url = format!("{}/api/v1/apis", self.management_api_url);
        let value = self.get(url).await?;
        parse_api_list(value)
    }

    pub async fn get_api_schema(&self, endpoint: &str) -> Result<Value> {
        let endpoint = normalized_segment(endpoint, "endpoint")?;
        let url = format!("{}/api/v1/apis/{endpoint}", self.management_api_url);
        let response = self
            .http
            .get(url)
            .header("X-MICROCMS-API-KEY", &self.api_key)
            .send()
            .await
            .context("microCMS Management API schema request failed")?;
        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<response body unavailable>".to_string());
            bail!(
                "microCMS Management API returned HTTP {status}: {}",
                body_snippet(&body)
            );
        }
        response
            .json()
            .await
            .context("failed to decode the microCMS API schema response")
    }

    pub async fn get_content_collection(
        &self,
        endpoint: &str,
        limit: usize,
        offset: usize,
        query: &ContentQuery,
        expected_kind: Option<ContentCollectionKind>,
    ) -> Result<ContentCollection> {
        let endpoint = endpoint.trim().trim_matches('/');
        if endpoint.is_empty() {
            bail!("endpoint is missing or empty");
        }

        let url = format!("{}/api/v1/{endpoint}", self.content_api_url);
        let mut request = self
            .http
            .get(url)
            .header("X-MICROCMS-API-KEY", &self.api_key)
            .query(&[("limit", limit), ("offset", offset)]);
        let query_pairs = query.query_pairs();
        if !query_pairs.is_empty() {
            request = request.query(&query_pairs);
        }

        let response = request.send().await.context("microCMS request failed")?;
        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<response body unavailable>".to_string());
            bail!("microCMS returned HTTP {status}: {}", body_snippet(&body));
        }
        let value = response
            .json()
            .await
            .context("failed to decode the microCMS content response")?;
        parse_content_collection(value, limit, offset, expected_kind)
    }

    pub async fn get_content_version(
        &self,
        endpoint: &str,
        content_id: &str,
        query: &ContentQuery,
    ) -> Result<Value> {
        let endpoint = normalized_segment(endpoint, "endpoint")?;
        let content_id = normalized_segment(content_id, "content ID")?;
        let url = format!("{}/api/v1/{endpoint}/{content_id}", self.content_api_url);
        let mut request = self
            .http
            .get(url)
            .header("X-MICROCMS-API-KEY", &self.api_key);
        let query_pairs = query.query_pairs();
        if !query_pairs.is_empty() {
            request = request.query(&query_pairs);
        }
        let response = request
            .send()
            .await
            .context("microCMS content version request failed")?;
        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<response body unavailable>".to_string());
            bail!("microCMS returned HTTP {status}: {}", body_snippet(&body));
        }
        response
            .json()
            .await
            .context("failed to decode the microCMS content version response")
    }

    pub async fn create_content(
        &self,
        endpoint: &str,
        value: &Value,
        status: ContentWriteStatus,
    ) -> Result<()> {
        let endpoint = normalized_segment(endpoint, "endpoint")?;
        let url = format!("{}/api/v1/{endpoint}", self.content_api_url);
        let mut request = self
            .http
            .post(url)
            .header("X-MICROCMS-API-KEY", &self.api_key)
            .json(value);
        if let Some(status) = status.query_value() {
            request = request.query(&[("status", status)]);
        }
        send_mutation(request, "create content").await
    }

    pub async fn put_content(
        &self,
        endpoint: &str,
        content_id: &str,
        value: &Value,
        status: ContentWriteStatus,
    ) -> Result<()> {
        let endpoint = normalized_segment(endpoint, "endpoint")?;
        let content_id = normalized_segment(content_id, "content ID")?;
        let url = format!("{}/api/v1/{endpoint}/{content_id}", self.content_api_url);
        let mut request = self
            .http
            .put(url)
            .header("X-MICROCMS-API-KEY", &self.api_key)
            .json(value);
        if let Some(status) = status.query_value() {
            request = request.query(&[("status", status)]);
        }
        send_mutation(request, "create content with specified ID").await
    }

    pub async fn list_content_metadata(
        &self,
        endpoint: &str,
        limit: usize,
        offset: usize,
    ) -> Result<ContentMetaList> {
        let endpoint = normalized_segment(endpoint, "endpoint")?;
        let url = format!("{}/api/v1/contents/{endpoint}", self.management_api_url);
        let response = self
            .http
            .get(url)
            .header("X-MICROCMS-API-KEY", &self.api_key)
            .query(&[("limit", limit), ("offset", offset)])
            .send()
            .await
            .context("microCMS Management API content metadata request failed")?;
        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<response body unavailable>".to_string());
            bail!(
                "microCMS Management API returned HTTP {status}: {}",
                body_snippet(&body)
            );
        }
        response
            .json()
            .await
            .context("failed to decode the microCMS content metadata response")
    }

    pub async fn get_content_metadata(
        &self,
        endpoint: &str,
        content_id: &str,
    ) -> Result<ContentMeta> {
        let endpoint = normalized_segment(endpoint, "endpoint")?;
        let content_id = normalized_segment(content_id, "content ID")?;
        let url = format!(
            "{}/api/v1/contents/{endpoint}/{content_id}",
            self.management_api_url
        );
        let response = self
            .http
            .get(url)
            .header("X-MICROCMS-API-KEY", &self.api_key)
            .send()
            .await
            .context("microCMS Management API content metadata request failed")?;
        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<response body unavailable>".to_string());
            bail!(
                "microCMS Management API returned HTTP {status}: {}",
                body_snippet(&body)
            );
        }
        response
            .json()
            .await
            .context("failed to decode the microCMS content metadata response")
    }

    pub async fn update_content(
        &self,
        endpoint: &str,
        content_id: &str,
        value: &Value,
        status: ContentWriteStatus,
    ) -> Result<()> {
        let endpoint = normalized_segment(endpoint, "endpoint")?;
        let content_id = normalized_segment(content_id, "content ID")?;
        let url = format!("{}/api/v1/{endpoint}/{content_id}", self.content_api_url);
        let mut request = self
            .http
            .patch(url)
            .header("X-MICROCMS-API-KEY", &self.api_key)
            .json(value);
        if let Some(status) = status.query_value() {
            request = request.query(&[("status", status)]);
        }
        send_mutation(request, "update content").await
    }

    pub async fn delete_content(&self, endpoint: &str, content_id: &str) -> Result<()> {
        let endpoint = normalized_segment(endpoint, "endpoint")?;
        let content_id = normalized_segment(content_id, "content ID")?;
        let url = format!("{}/api/v1/{endpoint}/{content_id}", self.content_api_url);
        let request = self
            .http
            .delete(url)
            .header("X-MICROCMS-API-KEY", &self.api_key);
        send_mutation(request, "delete content").await
    }

    pub async fn update_publication_status(
        &self,
        endpoint: &str,
        content_id: &str,
        status: PublicationStatus,
    ) -> Result<()> {
        let endpoint = normalized_segment(endpoint, "endpoint")?;
        let content_id = normalized_segment(content_id, "content ID")?;
        let url = format!(
            "{}/api/v1/contents/{endpoint}/{content_id}/status",
            self.management_api_url
        );
        let request = self
            .http
            .patch(url)
            .header("X-MICROCMS-API-KEY", &self.api_key)
            .json(&serde_json::json!({"status": [status.api_value()]}));
        send_mutation(request, "change content publication status").await
    }

    pub async fn update_reservation(
        &self,
        endpoint: &str,
        content_id: &str,
        publish_time: Option<&str>,
        stop_time: Option<&str>,
    ) -> Result<()> {
        let endpoint = normalized_segment(endpoint, "endpoint")?;
        let content_id = normalized_segment(content_id, "content ID")?;
        let url = format!(
            "{}/api/v1/contents/{endpoint}/{content_id}/reservation",
            self.management_api_url
        );
        let body = reservation_body(publish_time, stop_time);
        let request = self
            .http
            .put(url)
            .header("X-MICROCMS-API-KEY", &self.api_key)
            .json(&body);
        send_mutation(request, "update content publication reservation").await
    }

    async fn get(&self, url: String) -> Result<Value> {
        let response = self
            .http
            .get(url)
            .header("X-MICROCMS-API-KEY", &self.api_key)
            .send()
            .await
            .context("microCMS Management API request failed")?;
        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<response body unavailable>".to_string());
            bail!(
                "microCMS Management API returned HTTP {status}: {}",
                body_snippet(&body)
            );
        }
        response
            .json()
            .await
            .context("failed to decode the microCMS API list response")
    }
}

fn reservation_body(publish_time: Option<&str>, stop_time: Option<&str>) -> Value {
    let mut body = serde_json::Map::new();
    if let Some(publish_time) = publish_time {
        body.insert("publishTime".into(), Value::String(publish_time.into()));
    }
    if let Some(stop_time) = stop_time {
        body.insert("stopTime".into(), Value::String(stop_time.into()));
    }
    Value::Object(body)
}

fn parse_content_collection(
    value: Value,
    requested_limit: usize,
    requested_offset: usize,
    expected_kind: Option<ContentCollectionKind>,
) -> Result<ContentCollection> {
    let object = value
        .as_object()
        .context("microCMS content response must be a JSON object")?;

    let has_complete_list_metadata = object.get("totalCount").and_then(Value::as_u64).is_some()
        && object.get("offset").and_then(Value::as_u64).is_some()
        && object.get("limit").and_then(Value::as_u64).is_some();
    let contents = object.get("contents").and_then(Value::as_array);
    let treat_as_list = match expected_kind {
        Some(ContentCollectionKind::List) => contents.is_some(),
        Some(ContentCollectionKind::Object) => false,
        None => contents.is_some() && has_complete_list_metadata,
    };
    if treat_as_list {
        let Some(contents) = contents else {
            bail!("list response does not contain a contents array");
        };
        let offset = object
            .get("offset")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(requested_offset);
        let limit = object
            .get("limit")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(requested_limit);
        let total_count = object
            .get("totalCount")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or_else(|| offset.saturating_add(contents.len()));
        return Ok(ContentCollection {
            kind: ContentCollectionKind::List,
            total_count,
            offset,
            limit,
            contents: contents.clone(),
        });
    }

    Ok(ContentCollection {
        kind: ContentCollectionKind::Object,
        total_count: 1,
        offset: 0,
        limit: 1,
        contents: vec![value],
    })
}

fn nonempty(value: &Option<String>) -> Option<&str> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn api_urls_from_env(service_id: &str) -> Result<(String, String)> {
    resolve_api_urls(
        service_id,
        std::env::var("MICROCMS_CONTENT_API_URL").ok(),
        std::env::var("MICROCMS_MANAGEMENT_API_URL").ok(),
    )
}

fn resolve_api_urls(
    service_id: &str,
    content_domain: Option<String>,
    management_domain: Option<String>,
) -> Result<(String, String)> {
    let resolve = |name: &str, configured: Option<String>, default: &str| {
        let domain = configured
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| default.to_string());
        service_api_url(service_id, &domain)
            .with_context(|| format!("invalid {name} environment variable"))
    };
    Ok((
        resolve(
            "MICROCMS_CONTENT_API_URL",
            content_domain,
            "https://microcms.io",
        )?,
        resolve(
            "MICROCMS_MANAGEMENT_API_URL",
            management_domain,
            "https://microcms-management.io",
        )?,
    ))
}

fn service_api_url(service_id: &str, base_url: &str) -> Result<String> {
    let base_url = base_url.trim().trim_end_matches('/');
    let base_url = if base_url.contains("://") {
        base_url.to_string()
    } else {
        format!("https://{base_url}")
    };
    let mut url = reqwest::Url::parse(&base_url).context("API URL is not valid")?;
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        bail!("API URL must not include a path, query, or fragment");
    }
    let host = url.host_str().context("API URL host is missing")?;
    let service_host = format!("{service_id}.{host}");
    url.set_host(Some(&service_host))
        .map_err(|_| anyhow::anyhow!("service ID and API host do not form a valid hostname"))?;
    Ok(url.as_str().trim_end_matches('/').to_string())
}

fn normalized_segment<'a>(value: &'a str, name: &str) -> Result<&'a str> {
    let value = value.trim().trim_matches('/');
    if value.is_empty() {
        bail!("{name} is missing or empty");
    }
    Ok(value)
}

async fn send_mutation(request: reqwest::RequestBuilder, operation: &str) -> Result<()> {
    let response = request
        .send()
        .await
        .with_context(|| format!("microCMS request failed while attempting to {operation}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<response body unavailable>".to_string());
        bail!(
            "microCMS returned HTTP {status} while attempting to {operation}: {}",
            body_snippet(&body)
        );
    }
    Ok(())
}

fn body_snippet(body: &str) -> String {
    body.chars().take(500).collect()
}

fn parse_api_list(value: Value) -> Result<ApiList> {
    let entries = match value {
        Value::Array(entries) => entries,
        Value::Object(mut object) => match object.remove("apis") {
            Some(Value::Array(entries)) => entries,
            _ => bail!("microCMS API list response does not contain an 'apis' array"),
        },
        _ => bail!("microCMS API list response must be an array or object"),
    };

    let apis = entries
        .into_iter()
        .filter_map(|entry| {
            let object = entry.as_object()?;
            let endpoint = string_field(object, &["endpoint", "apiId", "id"])?;
            if endpoint.trim().is_empty() {
                return None;
            }
            Some(ApiInfo {
                endpoint: endpoint.to_string(),
                name: string_field(object, &["name", "displayName"]).map(str::to_string),
                description: string_field(object, &["description"]).map(str::to_string),
            })
        })
        .collect();
    Ok(ApiList { apis })
}

fn string_field<'a>(
    object: &'a serde_json::Map<String, Value>,
    fields: &[&str],
) -> Option<&'a str> {
    fields
        .iter()
        .find_map(|field| object.get(*field).and_then(Value::as_str))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn content_query_ignores_empty_values() {
        assert!(!ContentQuery::default().has_query());
        assert!(!ContentQuery {
            q: Some(String::new()),
            filters: None,
            orders: None,
            ..ContentQuery::default()
        }
        .has_query());
        assert!(ContentQuery {
            q: Some("keyword".into()),
            filters: None,
            orders: None,
            ..ContentQuery::default()
        }
        .has_query());
    }

    #[test]
    fn content_query_includes_only_configured_extended_parameters() {
        let query = ContentQuery {
            fields: Some("title,author.name".into()),
            depth: Some(2),
            ids: Some("one,two".into()),
            draft_key: Some("draft-key".into()),
            rich_editor_format: Some("object".into()),
            ..ContentQuery::default()
        };
        assert_eq!(
            query.query_pairs(),
            vec![
                ("fields", "title,author.name".into()),
                ("ids", "one,two".into()),
                ("draftKey", "draft-key".into()),
                ("richEditorFormat", "object".into()),
                ("depth", "2".into()),
            ]
        );
        assert!(query.has_query());
        assert!(ContentQuery {
            fields: Some("  ".into()),
            ..ContentQuery::default()
        }
        .query_pairs()
        .is_empty());
    }

    #[test]
    fn reservation_body_supports_start_end_both_and_clear() {
        assert_eq!(
            reservation_body(Some("start"), Some("stop")),
            json!({"publishTime": "start", "stopTime": "stop"})
        );
        assert_eq!(
            reservation_body(Some("start"), None),
            json!({"publishTime": "start"})
        );
        assert_eq!(
            reservation_body(None, Some("stop")),
            json!({"stopTime": "stop"})
        );
        assert_eq!(reservation_body(None, None), json!({}));
    }

    #[test]
    fn publication_status_uses_official_api_values() {
        assert_eq!(PublicationStatus::Publish.api_value(), "PUBLISH");
        assert_eq!(PublicationStatus::Draft.api_value(), "DRAFT");
    }

    #[test]
    fn content_write_status_only_adds_query_for_draft() {
        assert_eq!(ContentWriteStatus::Default.query_value(), None);
        assert_eq!(ContentWriteStatus::Draft.query_value(), Some("draft"));
    }

    #[test]
    fn api_url_override_keeps_service_id_out_of_environment_value() {
        assert_eq!(
            service_api_url("service", "https://microcms-staging.net/").unwrap(),
            "https://service.microcms-staging.net"
        );
        assert_eq!(
            service_api_url("service", "microcms.io").unwrap(),
            "https://service.microcms.io"
        );
        assert!(service_api_url("service", "https://microcms.io/api/v1").is_err());
    }

    #[test]
    fn resolves_default_content_and_management_api_urls() {
        assert_eq!(
            resolve_api_urls("service", None, None).unwrap(),
            (
                "https://service.microcms.io".into(),
                "https://service.microcms-management.io".into()
            )
        );
    }

    #[test]
    fn resolves_overridden_domains_for_both_apis_without_changing_service_id() {
        assert_eq!(
            resolve_api_urls(
                "same-service",
                Some("https://microcms-staging.net".into()),
                Some("https://microcms-management-staging.net".into())
            )
            .unwrap(),
            (
                "https://same-service.microcms-staging.net".into(),
                "https://same-service.microcms-management-staging.net".into()
            )
        );
    }

    #[test]
    fn empty_api_url_overrides_use_normal_domains() {
        assert_eq!(
            resolve_api_urls("service", Some("  ".into()), Some(String::new())).unwrap(),
            (
                "https://service.microcms.io".into(),
                "https://service.microcms-management.io".into()
            )
        );
    }

    #[test]
    fn parses_list_content_collection() {
        let collection = parse_content_collection(
            json!({
                "contents": [{"id": "one"}, {"id": "two"}],
                "totalCount": 12,
                "offset": 4,
                "limit": 2
            }),
            20,
            0,
            None,
        )
        .unwrap();

        assert_eq!(collection.kind, ContentCollectionKind::List);
        assert_eq!(collection.total_count, 12);
        assert_eq!(collection.offset, 4);
        assert_eq!(collection.limit, 2);
        assert_eq!(collection.contents.len(), 2);
    }

    #[test]
    fn confirmed_list_uses_contents_array_and_requested_pagination_fallback() {
        let collection = parse_content_collection(
            json!({"contents": [{"id": "only-match", "title": "Matched"}]}),
            20,
            0,
            Some(ContentCollectionKind::List),
        )
        .unwrap();

        assert_eq!(collection.kind, ContentCollectionKind::List);
        assert_eq!(
            collection.contents,
            vec![json!({"id": "only-match", "title": "Matched"})]
        );
        assert_eq!(collection.limit, 20);
        assert_eq!(collection.offset, 0);
        assert_eq!(collection.total_count, 1);
    }

    #[test]
    fn confirmed_object_endpoint_is_not_reclassified_by_user_contents_fields() {
        let value = json!({
            "contents": [{"label": "user field"}],
            "totalCount": 1,
            "offset": 0,
            "limit": 1
        });
        let collection =
            parse_content_collection(value.clone(), 20, 0, Some(ContentCollectionKind::Object))
                .unwrap();

        assert_eq!(collection.kind, ContentCollectionKind::Object);
        assert_eq!(collection.contents, vec![value]);
        assert_eq!(collection.limit, 1);
    }

    #[test]
    fn parses_object_content_collection_without_contents_array() {
        let value = json!({
            "title": "About",
            "totalCount": 99,
            "contents": "a user-defined non-array field"
        });
        let collection = parse_content_collection(value.clone(), 20, 0, None).unwrap();

        assert_eq!(collection.kind, ContentCollectionKind::Object);
        assert_eq!(collection.total_count, 1);
        assert_eq!(collection.offset, 0);
        assert_eq!(collection.limit, 1);
        assert_eq!(collection.contents, vec![value]);

        let user_contents_array = json!({"contents": ["user value"], "title": "Object"});
        let collection =
            parse_content_collection(user_contents_array.clone(), 20, 0, None).unwrap();
        assert_eq!(collection.kind, ContentCollectionKind::Object);
        assert_eq!(collection.contents, vec![user_contents_array]);
    }

    #[test]
    fn rejects_non_object_content_collection() {
        assert!(parse_content_collection(json!([]), 20, 0, None).is_err());
        assert!(parse_content_collection(json!("content"), 20, 0, None).is_err());
    }

    #[test]
    fn parses_management_content_metadata_response() {
        let metadata: ContentMetaList = serde_json::from_value(json!({
            "contents": [
                {
                    "id": "published",
                    "status": ["PUBLISH"],
                    "draftKey": "draft-key",
                    "reservationTime": {
                        "publishTime": "2026-08-01T00:00:00Z",
                        "stopTime": "2026-08-31T14:59:00Z"
                    }
                },
                {"id": "unknown"}
            ],
            "totalCount": 2,
            "offset": 0,
            "limit": 20
        }))
        .unwrap();

        assert_eq!(metadata.total_count, 2);
        assert_eq!(metadata.offset, 0);
        assert_eq!(metadata.limit, 20);
        assert_eq!(metadata.contents[0].status, vec!["PUBLISH".to_string()]);
        assert_eq!(metadata.contents[0].draft_key.as_deref(), Some("draft-key"));
        assert_eq!(
            metadata.contents[0]
                .reservation_time
                .as_ref()
                .and_then(|value| value.publish_time.as_deref()),
            Some("2026-08-01T00:00:00Z")
        );
        assert!(metadata.contents[1].status.is_empty());
    }

    #[test]
    fn parses_api_list_object_shape() {
        let parsed = parse_api_list(json!({
            "apis": [
                {"endpoint": "blogs", "name": "Blog posts", "description": "Posts"},
                {"apiId": "news", "displayName": "News", "extra": true}
            ],
            "other": "ignored"
        }))
        .unwrap();

        assert_eq!(
            parsed.apis,
            vec![
                ApiInfo {
                    endpoint: "blogs".into(),
                    name: Some("Blog posts".into()),
                    description: Some("Posts".into()),
                },
                ApiInfo {
                    endpoint: "news".into(),
                    name: Some("News".into()),
                    description: None,
                }
            ]
        );
    }

    #[test]
    fn parses_top_level_api_array() {
        let parsed = parse_api_list(json!([
            {"id": "authors", "name": "Authors"},
            {"unrecognized": "entry is skipped"}
        ]))
        .unwrap();

        assert_eq!(
            parsed.apis,
            vec![ApiInfo {
                endpoint: "authors".into(),
                name: Some("Authors".into()),
                description: None,
            }]
        );
    }
}
