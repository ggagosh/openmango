use anyhow::{Result, anyhow};
use uuid::Uuid;

use crate::state::{
    CollectionSubview, ForgeTabKey, QueryContent, QueryDefinition, QueryHistoryEntry,
    QueryImportReport, QueryLibrary, QueryLibraryPersistenceError, SavedQuery, SavedQueryInput,
    SessionKey,
};

use super::AppState;

impl AppState {
    pub fn query_history(&self) -> &[QueryHistoryEntry] {
        self.query_library.history()
    }

    pub fn saved_queries(&self) -> &[SavedQuery] {
        self.query_library.saved()
    }

    pub fn record_forge_query(&mut self, key: &ForgeTabKey, statement: String) -> Result<bool> {
        self.record_query(QueryDefinition {
            connection_id: key.connection_id,
            database: key.database.clone(),
            collection: None,
            content: QueryContent::Forge { statement },
        })
    }

    pub fn record_query(&mut self, definition: QueryDefinition) -> Result<bool> {
        let mut next = self.query_library.clone();
        if !next.record(definition) {
            return Ok(false);
        }
        self.query_library = next;
        self.persist_query_library()?;
        Ok(true)
    }

    pub fn save_query(&mut self, definition: QueryDefinition, name: &str) -> Result<Uuid> {
        self.update_query_library(|library| library.save(definition, name))
    }

    pub fn save_query_input(&mut self, input: SavedQueryInput) -> Result<Uuid> {
        self.update_query_library(|library| library.save_input(input))
    }

    pub fn edit_saved_query(&mut self, id: Uuid, input: SavedQueryInput) -> Result<()> {
        self.update_query_library(|library| library.edit_saved(id, input))
    }

    pub fn preview_saved_query_import(
        &self,
        inputs: &[SavedQueryInput],
    ) -> Result<QueryImportReport> {
        self.query_library.preview_import(inputs).map_err(anyhow::Error::msg)
    }

    pub fn import_saved_queries(
        &mut self,
        inputs: Vec<SavedQueryInput>,
    ) -> Result<QueryImportReport> {
        if self.query_library_persistence_blocked {
            return Err(QueryLibraryPersistenceError(
                "Saved queries were not imported because query_library.json is invalid. Fix or remove the file, then restart OpenMango."
                    .to_string(),
            )
            .into());
        }
        let mut next = self.query_library.clone();
        let report = next.import_saved(inputs).map_err(anyhow::Error::msg)?;
        self.config.save_query_library(&next).map_err(|error| {
            QueryLibraryPersistenceError(format!(
                "Saved queries were not imported because the library could not be saved: {error}"
            ))
        })?;
        self.query_library = next;
        Ok(report)
    }

    pub fn update_saved_query(&mut self, id: Uuid, definition: QueryDefinition) -> Result<()> {
        self.update_query_library(|library| library.update_saved(id, definition))
    }

    pub fn rename_saved_query(&mut self, id: Uuid, name: &str) -> Result<()> {
        self.update_query_library(|library| library.rename_saved(id, name))
    }

    pub fn duplicate_saved_query(&mut self, id: Uuid) -> Result<Uuid> {
        self.update_query_library(|library| library.duplicate_saved(id))
    }

    pub fn delete_history_query(&mut self, id: Uuid) -> Result<()> {
        self.update_query_library(|library| {
            if library.delete_history(id) {
                Ok(())
            } else {
                Err("That history entry no longer exists.".to_string())
            }
        })
    }

    pub fn clear_query_history(&mut self) -> Result<()> {
        self.update_query_library(|library| {
            library.clear_history();
            Ok(())
        })
    }

    pub fn delete_saved_query(&mut self, id: Uuid) -> Result<()> {
        self.update_query_library(|library| {
            if library.delete_saved(id) {
                Ok(())
            } else {
                Err("That saved query no longer exists.".to_string())
            }
        })
    }

    pub fn restore_document_query(
        &mut self,
        key: &SessionKey,
        definition: &QueryDefinition,
    ) -> Result<()> {
        let QueryContent::Documents(query) = &definition.content else {
            return Err(anyhow!("Open a Documents query before restoring this entry."));
        };
        self.set_filter(key, query.filter_raw.clone(), query.filter.clone());
        self.set_sort_projection(
            key,
            query.sort_raw.clone(),
            query.sort.clone(),
            query.projection_raw.clone(),
            query.projection.clone(),
        );
        self.set_collection_subview(key, CollectionSubview::Documents);
        Ok(())
    }

    pub fn restore_aggregation_query(
        &mut self,
        key: &SessionKey,
        definition: &QueryDefinition,
    ) -> Result<()> {
        let QueryContent::Aggregation { stages, selected_stage } = &definition.content else {
            return Err(anyhow!("Open the Aggregation view before restoring this entry."));
        };
        self.replace_pipeline_stages(key, stages.clone());
        self.set_pipeline_selected_stage(key, *selected_stage);
        self.set_collection_subview(key, CollectionSubview::Aggregation);
        Ok(())
    }

    pub fn restore_forge_query(
        &mut self,
        key: &ForgeTabKey,
        definition: &QueryDefinition,
    ) -> Result<()> {
        let QueryContent::Forge { statement } = &definition.content else {
            return Err(anyhow!("Open Forge before restoring this entry."));
        };
        self.set_forge_tab_content(key.id, statement.clone());
        Ok(())
    }

    pub fn has_query_library_target(&self) -> bool {
        match self.current_view {
            super::View::Documents => self.current_session_key().is_some_and(|key| {
                matches!(
                    self.session_subview(&key),
                    Some(CollectionSubview::Documents | CollectionSubview::Aggregation)
                )
            }),
            super::View::Forge => self.active_forge_tab_key().is_some(),
            _ => false,
        }
    }

    fn update_query_library<T>(
        &mut self,
        change: impl FnOnce(&mut QueryLibrary) -> std::result::Result<T, String>,
    ) -> Result<T> {
        let mut next = self.query_library.clone();
        let result = change(&mut next).map_err(anyhow::Error::msg)?;
        self.query_library = next;
        self.persist_query_library()?;
        Ok(result)
    }

    fn persist_query_library(&self) -> Result<()> {
        let error = if self.query_library_persistence_blocked {
            Some(
                "Query Library changed for this session only because query_library.json is invalid. Fix or remove the file, then restart OpenMango to resume persistence."
                    .to_string(),
            )
        } else {
            self.config
                .save_query_library(&self.query_library)
                .err()
                .map(|error| {
                    format!(
                        "Query Library changed for this session only because it could not be saved: {error}"
                    )
                })
        };

        match error {
            Some(error) => Err(QueryLibraryPersistenceError(error).into()),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use mongodb::bson::doc;

    use super::*;
    use crate::state::DocumentQuery;
    use crate::state::app_state::{ForgeTabState, PipelineStage};

    #[test]
    fn restores_document_query_fields_and_view() {
        let mut state = AppState::new();
        let key = SessionKey::new(Uuid::new_v4(), "app", "users");
        state.ensure_session(key.clone());
        let definition = QueryDefinition {
            connection_id: key.connection_id,
            database: key.database.clone(),
            collection: Some(key.collection.clone()),
            content: QueryContent::Documents(Box::new(DocumentQuery {
                filter_raw: "{ active: true }".into(),
                filter: Some(doc! { "active": true }),
                sort_raw: "{ name: 1 }".into(),
                sort: Some(doc! { "name": 1 }),
                projection_raw: "{ name: 1 }".into(),
                projection: Some(doc! { "name": 1 }),
            })),
        };

        state.restore_document_query(&key, &definition).unwrap();

        let data = state.session_data(&key).unwrap();
        assert_eq!(data.filter, Some(doc! { "active": true }));
        assert_eq!(data.sort, Some(doc! { "name": 1 }));
        assert_eq!(data.projection, Some(doc! { "name": 1 }));
        assert_eq!(state.session_subview(&key), Some(CollectionSubview::Documents));
    }

    #[test]
    fn restores_aggregation_pipeline_and_selection() {
        let mut state = AppState::new();
        let key = SessionKey::new(Uuid::new_v4(), "app", "users");
        state.ensure_session(key.clone());
        let stages = vec![PipelineStage {
            operator: "$match".into(),
            body: "{ active: true }".into(),
            enabled: true,
        }];
        let definition = QueryDefinition {
            connection_id: key.connection_id,
            database: key.database.clone(),
            collection: Some(key.collection.clone()),
            content: QueryContent::Aggregation { stages: stages.clone(), selected_stage: Some(0) },
        };

        state.restore_aggregation_query(&key, &definition).unwrap();

        let aggregation = &state.session_data(&key).unwrap().aggregation;
        assert_eq!(aggregation.stages, stages);
        assert_eq!(aggregation.selected_stage, Some(0));
        assert_eq!(state.session_subview(&key), Some(CollectionSubview::Aggregation));
    }

    #[test]
    fn blocked_persistence_rejects_import_without_mutating_memory() {
        let mut state = AppState::new();
        state.query_library_persistence_blocked = true;
        let before = state.query_library.clone();
        let result = state.import_saved_queries(vec![SavedQueryInput {
            name: "Users".into(),
            description: String::new(),
            tags: Vec::new(),
            scope: crate::state::SavedQueryScope::Connection,
            definition: QueryDefinition {
                connection_id: Uuid::new_v4(),
                database: "app".into(),
                collection: None,
                content: QueryContent::Forge { statement: "db.users.find({})".into() },
            },
        }]);
        assert!(result.is_err());
        assert_eq!(state.query_library, before);
    }

    #[test]
    fn restores_forge_statement() {
        let mut state = AppState::new();
        let key = ForgeTabKey {
            id: Uuid::new_v4(),
            connection_id: Uuid::new_v4(),
            database: "app".into(),
        };
        state.forge_tabs.insert(key.id, ForgeTabState::default());
        let definition = QueryDefinition {
            connection_id: key.connection_id,
            database: key.database.clone(),
            collection: None,
            content: QueryContent::Forge { statement: "db.users.find({})".into() },
        };

        state.restore_forge_query(&key, &definition).unwrap();

        assert_eq!(state.forge_tab_content(key.id), Some("db.users.find({})"));
    }
}
