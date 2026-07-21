use anyhow::{bail, Context, Result};
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct MicroCmsClient {
    service_id: String,
    api_key: String,
    http: reqwest::Client,
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
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContentQuery {
    pub q: Option<String>,
    pub filters: Option<String>,
    pub orders: Option<String>,
}

#[cfg(test)]
impl ContentQuery {
    pub fn has_query(&self) -> bool {
        [&self.q, &self.filters, &self.orders]
            .into_iter()
            .any(|value| value.as_deref().map_or(false, |value| !value.is_empty()))
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

        Ok(Self {
            service_id,
            api_key,
            http: reqwest::Client::new(),
        })
    }

    pub async fn list_apis(&self) -> Result<ApiList> {
        let url = format!(
            "https://{}.microcms-management.io/api/v1/apis",
            self.service_id
        );
        let value = self.get(url).await?;
        parse_api_list(value)
    }

    pub async fn get_api_schema(&self, endpoint: &str) -> Result<Value> {
        let endpoint = normalized_segment(endpoint, "endpoint")?;
        let url = format!(
            "https://{}.microcms-management.io/api/v1/apis/{endpoint}",
            self.service_id
        );
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
    ) -> Result<ContentCollection> {
        let endpoint = endpoint.trim().trim_matches('/');
        if endpoint.is_empty() {
            bail!("endpoint is missing or empty");
        }

        let url = format!(
            "https://{}.microcms.io/api/v1/{}",
            self.service_id, endpoint
        );
        let mut request = self
            .http
            .get(url)
            .header("X-MICROCMS-API-KEY", &self.api_key)
            .query(&[("limit", limit), ("offset", offset)]);
        if let Some(q) = nonempty(&query.q) {
            request = request.query(&[("q", q)]);
        }
        if let Some(filters) = nonempty(&query.filters) {
            request = request.query(&[("filters", filters)]);
        }
        if let Some(orders) = nonempty(&query.orders) {
            request = request.query(&[("orders", orders)]);
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
        parse_content_collection(value)
    }

    pub async fn create_content(
        &self,
        endpoint: &str,
        value: &Value,
        status: ContentWriteStatus,
    ) -> Result<()> {
        let endpoint = normalized_segment(endpoint, "endpoint")?;
        let url = format!("https://{}.microcms.io/api/v1/{endpoint}", self.service_id);
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
        let url = format!(
            "https://{}.microcms.io/api/v1/{endpoint}/{content_id}",
            self.service_id
        );
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
        let url = format!(
            "https://{}.microcms-management.io/api/v1/contents/{endpoint}",
            self.service_id
        );
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

    pub async fn update_content(
        &self,
        endpoint: &str,
        content_id: &str,
        value: &Value,
        status: ContentWriteStatus,
    ) -> Result<()> {
        let endpoint = normalized_segment(endpoint, "endpoint")?;
        let content_id = normalized_segment(content_id, "content ID")?;
        let url = format!(
            "https://{}.microcms.io/api/v1/{endpoint}/{content_id}",
            self.service_id
        );
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
        let url = format!(
            "https://{}.microcms.io/api/v1/{endpoint}/{content_id}",
            self.service_id
        );
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
            "https://{}.microcms-management.io/api/v1/contents/{endpoint}/{content_id}/status",
            self.service_id
        );
        let request = self
            .http
            .patch(url)
            .header("X-MICROCMS-API-KEY", &self.api_key)
            .json(&serde_json::json!({"status": [status.api_value()]}));
        send_mutation(request, "change content publication status").await
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

fn parse_content_collection(value: Value) -> Result<ContentCollection> {
    let object = value
        .as_object()
        .context("microCMS content response must be a JSON object")?;

    if let (Some(contents), Some(total_count), Some(offset), Some(limit)) = (
        object.get("contents").and_then(Value::as_array),
        object.get("totalCount").and_then(Value::as_u64),
        object.get("offset").and_then(Value::as_u64),
        object.get("limit").and_then(Value::as_u64),
    ) {
        return Ok(ContentCollection {
            kind: ContentCollectionKind::List,
            total_count: usize::try_from(total_count)
                .context("list response field totalCount is too large")?,
            offset: usize::try_from(offset).context("list response field offset is too large")?,
            limit: usize::try_from(limit).context("list response field limit is too large")?,
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
    value.as_deref().filter(|value| !value.is_empty())
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
        }
        .has_query());
        assert!(ContentQuery {
            q: Some("keyword".into()),
            filters: None,
            orders: None,
        }
        .has_query());
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
    fn parses_list_content_collection() {
        let collection = parse_content_collection(json!({
            "contents": [{"id": "one"}, {"id": "two"}],
            "totalCount": 12,
            "offset": 4,
            "limit": 2
        }))
        .unwrap();

        assert_eq!(collection.kind, ContentCollectionKind::List);
        assert_eq!(collection.total_count, 12);
        assert_eq!(collection.offset, 4);
        assert_eq!(collection.limit, 2);
        assert_eq!(collection.contents.len(), 2);
    }

    #[test]
    fn parses_object_content_collection_without_contents_array() {
        let value = json!({
            "title": "About",
            "totalCount": 99,
            "contents": "a user-defined non-array field"
        });
        let collection = parse_content_collection(value.clone()).unwrap();

        assert_eq!(collection.kind, ContentCollectionKind::Object);
        assert_eq!(collection.total_count, 1);
        assert_eq!(collection.offset, 0);
        assert_eq!(collection.limit, 1);
        assert_eq!(collection.contents, vec![value]);

        let user_contents_array = json!({"contents": ["user value"], "title": "Object"});
        let collection = parse_content_collection(user_contents_array.clone()).unwrap();
        assert_eq!(collection.kind, ContentCollectionKind::Object);
        assert_eq!(collection.contents, vec![user_contents_array]);
    }

    #[test]
    fn rejects_non_object_content_collection() {
        assert!(parse_content_collection(json!([])).is_err());
        assert!(parse_content_collection(json!("content")).is_err());
    }

    #[test]
    fn parses_management_content_metadata_response() {
        let metadata: ContentMetaList = serde_json::from_value(json!({
            "contents": [
                {"id": "published", "status": ["PUBLISH"]},
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
