//! Document/Collection view component.

mod actions;
mod explain;
mod header;
mod node_meta;
mod pagination;
mod query;
mod query_completion;
mod schema_filter;
mod schema_filter_completion;
mod state;
mod types;
mod view;
mod view_model;

pub mod dialogs;
pub mod tree;
pub mod views;

pub use state::CollectionView;
