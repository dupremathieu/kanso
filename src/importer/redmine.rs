//! Read-only Redmine API primitives. Never expose request errors or API keys.
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde::Serialize;

use super::{Candidate, ProviderError};

const LIMIT: usize = 200;
const PROJECT_LIMIT: usize = 1000;
const MAX_BYTES: u64 = 2 * 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(30);

/// Normalize a Redmine deployment URL without resolving hostnames or following redirects.
pub fn canonical_url(input: &str) -> Result<String, ProviderError> {
    if input.len() > 460
        || input
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
        || input.contains(['?', '#', '%'])
    {
        return Err(ProviderError);
    }
    let uri: axum::http::Uri = input.parse().map_err(|_| ProviderError)?;
    let scheme = uri.scheme_str().ok_or(ProviderError)?.to_ascii_lowercase();
    if !matches!(scheme.as_str(), "http" | "https") {
        return Err(ProviderError);
    }
    let authority = uri.authority().ok_or(ProviderError)?;
    if authority.as_str().contains('@') || authority.host().is_empty() {
        return Err(ProviderError);
    }
    let host = authority.host().to_ascii_lowercase();
    let host = if host.starts_with('[') {
        let ip = host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<std::net::Ipv6Addr>()
            .map_err(|_| ProviderError)?;
        format!("[{ip}]")
    } else if !host
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-')
    {
        return Err(ProviderError);
    } else {
        let host = host.trim_end_matches('.');
        if host.is_empty() {
            return Err(ProviderError);
        }
        host.to_owned()
    };
    let suffix = authority
        .as_str()
        .strip_prefix(authority.host())
        .ok_or(ProviderError)?;
    let port = if suffix.is_empty() {
        None
    } else {
        Some(
            suffix
                .strip_prefix(':')
                .ok_or(ProviderError)?
                .parse::<u16>()
                .map_err(|_| ProviderError)?,
        )
    };
    if port == Some(0) {
        return Err(ProviderError);
    }
    let path = uri.path().trim_end_matches('/');
    if path.split('/').any(|s| s == "." || s == "..")
        || path.contains("//")
        || !path
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'/' | b'-' | b'_' | b'.' | b'~'))
    {
        return Err(ProviderError);
    }
    let port = match port {
        Some(80) if scheme == "http" => String::new(),
        Some(443) if scheme == "https" => String::new(),
        Some(port) => format!(":{port}"),
        None => String::new(),
    };
    Ok(format!("{scheme}://{host}{port}{path}"))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filters {
    pub project: Option<u64>,
    pub assignee: Option<u64>,
    pub state: State,
    pub watched: bool,
    pub either: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Open,
    Closed,
    All,
}

impl Filters {
    /// `me` is resolved to the authenticated user ID before constructing filters.
    pub fn parse(values: &HashMap<String, String>, me: u64) -> Result<Self, ProviderError> {
        if me == 0
            || values.keys().any(|k| {
                !["project", "assignee", "state", "watched", "operator"].contains(&k.as_str())
            })
        {
            return Err(ProviderError);
        }
        fn number(value: &str) -> Result<Option<u64>, ProviderError> {
            if value.is_empty() || value == "all" {
                return Ok(None);
            }
            let value = value.parse::<u64>().map_err(|_| ProviderError)?;
            if value == 0 {
                return Err(ProviderError);
            }
            Ok(Some(value))
        }
        let project = number(values.get("project").map_or("", String::as_str))?;
        let assignee = match values.get("assignee").map_or("", String::as_str) {
            "me" => Some(me),
            value => number(value)?,
        };
        let state = match values.get("state").map_or("open", String::as_str) {
            "open" => State::Open,
            "closed" => State::Closed,
            "all" => State::All,
            _ => return Err(ProviderError),
        };
        let watched = match values.get("watched").map_or("", String::as_str) {
            "" | "all" => false,
            "me" => true,
            _ => return Err(ProviderError),
        };
        let either = match values.get("operator").map_or("and", String::as_str) {
            "and" => false,
            "or" => true,
            _ => return Err(ProviderError),
        };
        Ok(Self {
            project,
            assignee,
            state,
            watched,
            either,
        })
    }

    fn queries(&self, me: u64) -> Vec<String> {
        let state = match self.state {
            State::Open => "open",
            State::Closed => "closed",
            State::All => "*",
        };
        let mut base = format!("status_id={state}");
        if let Some(project) = self.project {
            base.push_str(&format!("&project_id={project}&subproject_id=%21%2A"));
        }
        let assignee = self
            .assignee
            .map(|id| format!("&assigned_to_id={id}"))
            .unwrap_or_default();
        let watched = if self.watched {
            format!("&watcher_id={me}")
        } else {
            String::new()
        };
        if self.either && self.assignee.is_some() && self.watched {
            vec![format!("{base}{assignee}"), format!("{base}{watched}")]
        } else {
            vec![format!("{base}{assignee}{watched}")]
        }
    }
}

#[derive(Deserialize)]
struct Page<T> {
    total_count: usize,
    offset: usize,
    limit: usize,
    #[serde(flatten)]
    data: T,
}
#[derive(Deserialize)]
struct Issues {
    issues: Vec<Issue>,
}
#[derive(Deserialize)]
struct Issue {
    id: u64,
    subject: String,
    description: Option<String>,
    project: Option<Reference>,
    assigned_to: Option<Reference>,
    status: Option<IssueStatus>,
}

#[derive(Deserialize)]
struct Reference {
    id: u64,
}

#[derive(Deserialize)]
struct IssueStatus {
    is_closed: Option<bool>,
}

#[derive(Deserialize, Serialize)]
pub struct Account {
    pub id: u64,
    pub name: String,
}

#[derive(Deserialize)]
struct CurrentUser {
    user: User,
}

#[derive(Deserialize)]
struct User {
    id: u64,
    login: String,
    firstname: Option<String>,
    lastname: Option<String>,
}

#[derive(Deserialize, Serialize)]
pub struct Project {
    pub id: u64,
    pub name: String,
}

#[derive(Deserialize)]
struct Projects {
    projects: Vec<Project>,
}

/// A credential-bearing client; it intentionally implements neither Debug nor Serialize.
pub struct Client {
    base: String,
    key: String,
    agent: ureq::Agent,
}

impl Client {
    pub fn new(url: &str, key: &str) -> Result<Self, ProviderError> {
        if key.is_empty()
            || key.len() > 4096
            || key.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(ProviderError);
        }
        let config = ureq::Agent::config_builder()
            .max_redirects(0)
            .timeout_global(Some(DEADLINE))
            .build();
        Ok(Self {
            base: canonical_url(url)?,
            key: key.to_owned(),
            agent: ureq::Agent::new_with_config(config),
        })
    }

    fn get<T: serde::de::DeserializeOwned>(
        &self,
        endpoint: &str,
        deadline: Instant,
        budget: &mut u64,
    ) -> Result<T, ProviderError> {
        use std::io::Read;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or(ProviderError)?;
        let mut response = self
            .agent
            .get(format!("{}{endpoint}", self.base))
            .header("X-Redmine-API-Key", &self.key)
            .config()
            .timeout_global(Some(remaining))
            .build()
            .call()
            .map_err(|_| ProviderError)?;
        if !response.status().is_success() {
            return Err(ProviderError);
        }
        let mut bytes = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(*budget + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ProviderError)?;
        if bytes.len() as u64 > *budget {
            return Err(ProviderError);
        }
        *budget -= bytes.len() as u64;
        serde_json::from_slice(&bytes).map_err(|_| ProviderError)
    }

    pub fn current_user(&self) -> Result<Account, ProviderError> {
        let mut budget = MAX_BYTES;
        let user: CurrentUser = self.get(
            "/users/current.json",
            Instant::now() + DEADLINE,
            &mut budget,
        )?;
        if user.user.id == 0 || user.user.login.is_empty() {
            return Err(ProviderError);
        }
        if user.user.login.contains(&self.key)
            || user
                .user
                .firstname
                .as_ref()
                .is_some_and(|value| value.contains(&self.key))
            || user
                .user
                .lastname
                .as_ref()
                .is_some_and(|value| value.contains(&self.key))
        {
            return Err(ProviderError);
        }
        let name = format!(
            "{} {}",
            user.user.firstname.unwrap_or_default(),
            user.user.lastname.unwrap_or_default()
        )
        .trim()
        .to_owned();
        Ok(Account {
            id: user.user.id,
            name: if name.is_empty() {
                user.user.login
            } else {
                name
            },
        })
    }

    pub fn projects(&self) -> Result<Vec<Project>, ProviderError> {
        let deadline = Instant::now() + DEADLINE;
        let mut budget = MAX_BYTES;
        let mut projects = Vec::new();
        let mut seen = HashSet::new();
        let mut expected_total = None;
        loop {
            let offset = projects.len();
            let page: Page<Projects> = self.get(
                &format!("/projects.json?status=1&limit=100&offset={offset}"),
                deadline,
                &mut budget,
            )?;
            if expected_total.is_some_and(|total| total != page.total_count)
                || page.total_count > PROJECT_LIMIT
                || page.offset != offset
                || page.limit == 0
                || page.data.projects.len() > page.limit
                || page.data.projects.len() > page.total_count.saturating_sub(offset)
                || (page.data.projects.is_empty() && offset != page.total_count)
            {
                return Err(ProviderError);
            }
            expected_total = Some(page.total_count);
            for project in page.data.projects {
                if project.id == 0
                    || project.name.is_empty()
                    || project.name.contains(&self.key)
                    || !seen.insert(project.id)
                {
                    return Err(ProviderError);
                }
                projects.push(project);
            }
            if projects.len() == page.total_count {
                break;
            }
        }
        Ok(projects)
    }

    pub fn issues(&self, filters: &Filters, me: u64) -> Result<Vec<Candidate>, ProviderError> {
        let mut result = Vec::new();
        let mut seen = HashSet::new();
        let deadline = Instant::now() + DEADLINE;
        let mut budget = MAX_BYTES;
        for query in filters.queries(me) {
            let mut offset = 0;
            let mut expected_total = None;
            let mut branch_seen = HashSet::new();
            loop {
                let page: Page<Issues> = self.get(
                    &format!("/issues.json?{query}&limit=100&offset={offset}&sort=id"),
                    deadline,
                    &mut budget,
                )?;
                if expected_total.is_some_and(|total| total != page.total_count)
                    || page.total_count > LIMIT
                    || page.offset != offset
                    || page.limit == 0
                    || page.data.issues.len() > page.limit
                    || page.data.issues.len() > page.total_count.saturating_sub(offset)
                {
                    return Err(ProviderError);
                }
                expected_total = Some(page.total_count);
                let count = page.data.issues.len();
                for issue in page.data.issues {
                    if issue.id == 0
                        || !branch_seen.insert(issue.id)
                        || issue.subject.contains(&self.key)
                        || issue
                            .description
                            .as_ref()
                            .is_some_and(|text| text.contains(&self.key))
                        || match filters.state {
                            State::Open => {
                                issue.status.as_ref().and_then(|s| s.is_closed) == Some(true)
                            }
                            State::Closed => {
                                issue.status.as_ref().and_then(|s| s.is_closed) == Some(false)
                            }
                            State::All => false,
                        }
                        || filters
                            .project
                            .is_some_and(|id| issue.project.as_ref().map(|p| p.id) != Some(id))
                        || (query.contains("&assigned_to_id=")
                            && issue.assigned_to.as_ref().map(|p| p.id) != filters.assignee)
                    {
                        return Err(ProviderError);
                    }
                    if seen.insert(issue.id) {
                        let url = format!("{}/issues/{}", self.base, issue.id);
                        let body = issue.description.unwrap_or_default();
                        let header = format!("[Redmine #{}]({})", issue.id, url);
                        let title_invalid =
                            crate::api::tasks::validate_title(&issue.subject).is_err();
                        let oversized = header.len().saturating_add(body.len()).saturating_add(2)
                            > 64 * 1024
                            || issue.subject.len() > 4096;
                        let title = if issue.subject.len() > 4096 {
                            format!("Redmine issue #{} (title too large)", issue.id)
                        } else {
                            issue.subject
                        };
                        let description = if oversized {
                            "Redmine content exceeds import limits and was not copied.".to_owned()
                        } else if body.is_empty() {
                            header
                        } else {
                            format!("{header}\n\n{body}")
                        };
                        result.push(Candidate {
                            identity: format!("{}#{}", self.base, issue.id),
                            kind: "Issue".into(),
                            title,
                            description,
                            source_url: url,
                            invalid_reason: if oversized {
                                Some("Redmine content exceeds import limits; nothing was truncated or imported.".into())
                            } else if title_invalid {
                                Some("Redmine title is empty or exceeds the 200-character title limit.".into())
                            } else { None },
                        });
                        if result.len() > LIMIT {
                            return Err(ProviderError);
                        }
                    }
                }
                offset += count;
                if offset == page.total_count {
                    break;
                }
                if count == 0 || offset > page.total_count {
                    return Err(ProviderError);
                }
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn mock(responses: Vec<(u16, String)>) -> (Client, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0; 4096];
                let _ = stream.read(&mut request).unwrap();
                let _ = write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        (
            Client::new(&format!("http://{address}"), "public-test-key").unwrap(),
            handle,
        )
    }

    fn page(start: usize, end: usize, total: usize, offset: usize) -> String {
        let issues: Vec<_> = (start..end).map(|i| serde_json::json!({"id": i + 1, "subject": format!("issue {i}"), "description": "text", "status":{"is_closed":false}})).collect();
        serde_json::json!({"issues":issues, "total_count":total, "offset":offset, "limit":100})
            .to_string()
    }

    fn all_filters() -> Filters {
        Filters {
            project: None,
            assignee: None,
            state: State::Open,
            watched: false,
            either: false,
        }
    }

    #[test]
    fn canonicalizes_and_rejects_unsafe_urls() {
        assert_eq!(
            canonical_url("HTTP://EXAMPLE.COM:80/redmine/").unwrap(),
            "http://example.com/redmine"
        );
        for url in [
            "http://u:p@example.com",
            "http://example.com/a/../b",
            "http://example.com/a?b",
            "http://example.com/%2e",
            "http://example.com:abc",
            "file:///etc",
        ] {
            assert!(canonical_url(url).is_err(), "{url}");
        }
    }

    #[test]
    fn filters_constrain_both_or_branches() {
        let filters = Filters {
            project: Some(4),
            assignee: Some(8),
            state: State::Closed,
            watched: true,
            either: true,
        };
        assert_eq!(
            filters.queries(9),
            [
                "status_id=closed&project_id=4&subproject_id=%21%2A&assigned_to_id=8",
                "status_id=closed&project_id=4&subproject_id=%21%2A&watcher_id=9"
            ]
        );
        let mut filters = filters;
        filters.assignee = None;
        assert_eq!(
            filters.queries(9),
            ["status_id=closed&project_id=4&subproject_id=%21%2A&watcher_id=9"]
        );
        let mut values = HashMap::new();
        values.insert("assignee".into(), "me".into());
        values.insert("watched".into(), "me".into());
        values.insert("operator".into(), "and".into());
        let and = Filters::parse(&values, 9).unwrap();
        assert_eq!(
            and.queries(9),
            ["status_id=open&assigned_to_id=9&watcher_id=9"]
        );
        values.insert("operator".into(), "bad".into());
        assert!(Filters::parse(&values, 9).is_err());
    }

    #[test]
    fn union_is_complete_and_deduplicated() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for n in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut bytes = [0; 4096];
                let len = stream.read(&mut bytes).unwrap();
                let request = String::from_utf8_lossy(&bytes[..len]);
                assert!(
                    request.contains("X-Redmine-API-Key: public-test-key")
                        || request.contains("x-redmine-api-key: public-test-key")
                );
                assert!(request.contains("project_id=4"));
                assert!(request.contains("status_id=closed"));
                let body = if n == 0 {
                    r#"{"issues":[{"id":1,"subject":"first","description":"text","project":{"id":4},"assigned_to":{"id":8},"status":{"is_closed":true}}],"total_count":1,"offset":0,"limit":100}"#
                } else {
                    r#"{"issues":[{"id":1,"subject":"first","description":"text","project":{"id":4},"status":{"is_closed":true}},{"id":2,"subject":"second","description":null,"project":{"id":4},"status":{"is_closed":true}}],"total_count":2,"offset":0,"limit":100}"#
                };
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            }
        });
        let client = Client::new(&format!("http://{address}/redmine"), "public-test-key").unwrap();
        let filters = Filters {
            project: Some(4),
            assignee: Some(8),
            state: State::Closed,
            watched: true,
            either: true,
        };
        let result = client.issues(&filters, 9).unwrap();
        server.join().unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].title, "first");
        assert_eq!(
            result[0].source_url,
            format!("http://{address}/redmine/issues/1")
        );
    }

    #[test]
    fn pagination_retrieves_complete_bounded_result() {
        let (client, server) = mock(vec![
            (200, page(0, 100, 150, 0)),
            (200, page(100, 150, 150, 100)),
        ]);
        let issues = client.issues(&all_filters(), 1).unwrap();
        server.join().unwrap();
        assert_eq!(issues.len(), 150);
        assert_eq!(issues.last().unwrap().title, "issue 149");
    }

    #[test]
    fn errors_and_malformed_pagination_fail_closed() {
        for (status, body) in [
            (200, page(0, 1, 201, 0)),
            (200, page(0, 1, 1, 9)),
            (200, page(0, 0, 1, 0)),
            (503, "{}".to_owned()),
        ] {
            let (client, server) = mock(vec![(status, body)]);
            assert!(client.issues(&all_filters(), 1).is_err());
            server.join().unwrap();
        }
    }

    #[test]
    fn failed_union_branch_never_returns_partial_results() {
        let first = r#"{"issues":[{"id":1,"subject":"first","description":"","assigned_to":{"id":9},"status":{"is_closed":false}}],"total_count":1,"offset":0,"limit":100}"#;
        let (client, server) = mock(vec![(200, first.into()), (503, "{}".into())]);
        let filters = Filters {
            project: None,
            assignee: Some(9),
            state: State::Open,
            watched: true,
            either: true,
        };
        // This fixture must satisfy the assignee predicate for the first branch.
        // An error in either branch must reject the complete preview.
        assert!(client.issues(&filters, 9).is_err());
        server.join().unwrap();
    }

    #[test]
    fn expired_deadline_and_oversized_body_fail_closed() {
        let client = Client::new("http://127.0.0.1:1", "public-test-key").unwrap();
        let mut budget = MAX_BYTES;
        assert!(client
            .get::<serde_json::Value>("/issues.json", Instant::now(), &mut budget)
            .is_err());
        let (client, server) = mock(vec![(200, "x".repeat(MAX_BYTES as usize + 1))]);
        assert!(client.issues(&all_filters(), 1).is_err());
        server.join().unwrap();
    }

    #[test]
    fn invalid_content_is_individual_and_explained() {
        let body = serde_json::json!({"issues":[{"id":1,"subject":"a".repeat(5000),"description":"","status":{"is_closed":false}},{"id":2,"subject":"valid","description":"b".repeat(65536),"status":{"is_closed":false}}],"total_count":2,"offset":0,"limit":100}).to_string();
        let (client, server) = mock(vec![(200, body)]);
        let items = client.issues(&all_filters(), 1).unwrap();
        server.join().unwrap();
        assert!(items.iter().all(|item| item.invalid_reason.is_some()));
        assert!(items.iter().all(|item| item.description.len() < 1024));
    }

    #[test]
    fn upstream_cannot_reflect_a_key_into_candidates() {
        let body = r#"{"issues":[{"id":1,"subject":"public-test-key","description":""}],"total_count":1,"offset":0,"limit":100}"#;
        let (client, server) = mock(vec![(200, body.into())]);
        assert!(client.issues(&all_filters(), 1).is_err());
        server.join().unwrap();
    }

    #[test]
    fn redirects_fail_without_following() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        target.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let target_address = target.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            let _ = stream.read(&mut request).unwrap();
            write!(stream, "HTTP/1.1 302 Found\r\nLocation: http://{target_address}/stolen\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        let client = Client::new(&format!("http://{address}"), "public-test-key").unwrap();
        assert!(client.issues(&all_filters(), 1).is_err());
        server.join().unwrap();
        assert_eq!(
            target.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn project_discovery_requests_only_active_projects() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = [0; 4096];
            let len = stream.read(&mut bytes).unwrap();
            let request = String::from_utf8_lossy(&bytes[..len]);
            assert!(
                request.starts_with("GET /projects.json?status=1&limit=100&offset=0 "),
                "{request}"
            );
            let body =
                r#"{"projects":[{"id":4,"name":"Atlas"}],"total_count":1,"offset":0,"limit":100}"#;
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        });
        let client = Client::new(&format!("http://{address}"), "public-test-key").unwrap();
        let projects = client.projects().unwrap();
        server.join().unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].name, "Atlas");
    }

    #[test]
    fn project_discovery_paginates_and_rejects_duplicates() {
        let projects: Vec<_> = (1..=100)
            .map(|id| serde_json::json!({"id":id,"name":format!("P{id}")}))
            .collect();
        let first =
            serde_json::json!({"projects":projects,"total_count":101,"offset":0,"limit":100})
                .to_string();
        let second = serde_json::json!({"projects":[{"id":101,"name":"P101"}],"total_count":101,"offset":100,"limit":100}).to_string();
        let (client, server) = mock(vec![(200, first.clone()), (200, second)]);
        assert_eq!(client.projects().unwrap().len(), 101);
        server.join().unwrap();
        let duplicate = serde_json::json!({"projects":[{"id":1,"name":"P1"}],"total_count":101,"offset":100,"limit":100}).to_string();
        let (client, server) = mock(vec![(200, first), (200, duplicate)]);
        assert!(client.projects().is_err());
        server.join().unwrap();
    }

    #[test]
    fn discovery_rejects_reflected_keys() {
        let account = r#"{"user":{"id":1,"login":"public-test-key"}}"#;
        let (client, server) = mock(vec![(200, account.into())]);
        assert!(client.current_user().is_err());
        server.join().unwrap();
        let projects = r#"{"projects":[{"id":1,"name":"public-test-key"}],"total_count":1,"offset":0,"limit":100}"#;
        let (client, server) = mock(vec![(200, projects.into())]);
        assert!(client.projects().is_err());
        server.join().unwrap();
    }

    #[test]
    fn contradictory_project_and_state_results_are_rejected() {
        let wrong_state = r#"{"issues":[{"id":1,"subject":"Closed","status":{"is_closed":true}}],"total_count":1,"offset":0,"limit":100}"#;
        let (client, server) = mock(vec![(200, wrong_state.into())]);
        assert!(client.issues(&all_filters(), 1).is_err());
        server.join().unwrap();
        let wrong_project = r#"{"issues":[{"id":1,"subject":"Other project","project":{"id":2},"status":{"is_closed":false}}],"total_count":1,"offset":0,"limit":100}"#;
        let (client, server) = mock(vec![(200, wrong_project.into())]);
        let mut filters = all_filters();
        filters.project = Some(1);
        assert!(client.issues(&filters, 1).is_err());
        server.join().unwrap();
    }
}
