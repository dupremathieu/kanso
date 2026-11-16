//! Provider bridge for browser-managed Redmine connections.
use std::{collections::HashMap, sync::Arc};

use super::{redmine::Filters, Candidate, Filter, Provider, ProviderError, ProviderInfo, Source};
use crate::redmine_connections::Connections;

pub const ID: &str = "redmine";

pub struct Redmine {
    pub connections: Arc<Connections>,
}

impl Provider for Redmine {
    fn source_revision(&self, source: &str) -> Option<String> {
        self.connections
            .get(source)
            .map(|connection| connection.revision)
    }
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: ID.into(),
            name: "Redmine".into(),
            sources: self
                .connections
                .list()
                .into_iter()
                .map(|c| Source {
                    id: c.id,
                    name: c.name,
                })
                .collect(),
            filters: ["project", "assignee", "state", "watched", "operator"]
                .into_iter()
                .map(|id| Filter {
                    id: id.into(),
                    label: id.into(),
                })
                .collect(),
        }
    }

    fn failure_message(&self) -> &'static str {
        "Redmine preview failed or exceeded limits; check the connection and narrow the filters"
    }

    fn preview(
        &self,
        source: &str,
        filters: &HashMap<String, String>,
    ) -> Result<Vec<Candidate>, ProviderError> {
        let connection = self.connections.get(source).ok_or(ProviderError)?;
        let user = connection.client.current_user()?;
        let filters = Filters::parse(filters, user.id)?;
        connection.client.issues(&filters, user.id)
    }
}
