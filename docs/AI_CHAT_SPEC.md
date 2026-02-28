# OpenMango AI Chat Feature — Developer Specification

> **Version:** 1.0  
> **Date:** February 2026  
> **Status:** Draft  
> **Repo:** https://github.com/ggagosh/openmango

---

## Table of Contents

1. [Overview](#1-overview)
2. [Architecture](#2-architecture)
3. [LLM Provider Layer — Rig Framework](#3-llm-provider-layer--rig-framework)
4. [Tokio ↔ GPUI Runtime Bridge](#4-tokio--gpui-runtime-bridge)
5. [Agent Loop](#5-agent-loop)
6. [Tool Definitions](#6-tool-definitions)
7. [Gen UI — Chat Block System](#7-gen-ui--chat-block-system)
8. [Context Assembly](#8-context-assembly)
9. [MongoDB Query Generation — Prompt Engineering](#9-mongodb-query-generation--prompt-engineering)
10. [Safety and Permission Model](#10-safety-and-permission-model)
11. [Streaming and UI Integration](#11-streaming-and-ui-integration)
12. [API Key Management](#12-api-key-management)
13. [Configuration and Settings](#13-configuration-and-settings)
14. [File Structure](#14-file-structure)
15. [Implementation Phases](#15-implementation-phases)
16. [Appendix A — Provider API Reference](#appendix-a--provider-api-reference)
17. [Appendix B — BSON Type Mapping](#appendix-b--bson-type-mapping)

---

## 1. Overview

OpenMango is a GPU-accelerated MongoDB client for macOS, built in Rust with GPUI. This specification defines an AI Chat feature that enables users to interact with their MongoDB databases through natural language. The chat is not a simple query generator — it is an agentic, multi-turn interface with structured outputs that map to native GPUI components (Gen UI pattern), capable of executing read operations automatically and write operations with user confirmation.

### Design Principles

- **Rig as the LLM backbone.** Use the `rig-core` crate for provider abstraction, tool calling, and streaming. Do not reimplement provider-specific serialization.
- **Agent loop is ours.** Rig handles LLM communication; OpenMango owns the orchestration loop, safety checks, and UI rendering.
- **Gen UI, not chat-as-text.** The LLM selects from a catalog of typed `ChatBlock` components. GPUI renders them as native, interactive elements.
- **Local-first privacy.** Schema sampling and prompt construction happen locally. Only the assembled prompt goes to the LLM. User data never leaves the machine unless the user explicitly runs a query.
- **Multi-provider from day one.** Abstract over Anthropic, OpenAI, Gemini, and Ollama via Rig's provider system. Users bring their own API keys.

### Competitive Context

MongoDB Compass uses Azure OpenAI for natural language → query generation with a review-before-execute pattern. It is locked to a single provider, has no agentic multi-turn capability, and cannot render structured UI beyond query strings. OpenMango's chat will differentiate by offering multi-provider support, multi-turn agentic conversations, structured Gen UI rendering, and deep integration with the existing tabbed workspace.

---

## 2. Architecture

### High-Level Data Flow

```
┌─────────────────────────────────────────────────────────┐
│                    GPUI Main Thread                       │
│                                                           │
│  ┌──────────┐     ┌──────────┐     ┌──────────────────┐ │
│  │ ChatPanel │────▶│ AgentLoop│────▶│ ChatBlock Render  │ │
│  │  (View)   │◀────│          │◀────│  (GPUI Elements)  │ │
│  └──────────┘     └────┬─────┘     └──────────────────┘ │
│                        │                                  │
│                   ┌────▼─────┐                           │
│                   │  Bridge  │  cx.spawn → tokio_rt.spawn│
│                   └────┬─────┘                           │
└────────────────────────┼────────────────────────────────┘
                         │
┌────────────────────────┼────────────────────────────────┐
│              Tokio Runtime (Background)                   │
│                        │                                  │
│  ┌─────────────────────▼──────────────────────────────┐ │
│  │              Rig Agent + Tools                       │ │
│  │                                                      │ │
│  │  ┌───────────┐  ┌──────────┐  ┌─────────────────┐  │ │
│  │  │ Anthropic  │  │  OpenAI  │  │  Gemini / Ollama│  │ │
│  │  │  Provider  │  │ Provider │  │    Provider     │  │ │
│  │  └───────────┘  └──────────┘  └─────────────────┘  │ │
│  │                                                      │ │
│  │  ┌───────────┐  ┌──────────┐  ┌─────────────────┐  │ │
│  │  │ RunFind   │  │RunAggr.  │  │ GetSchema  ...  │  │ │
│  │  │  Tool     │  │  Tool    │  │    Tool         │  │ │
│  │  └───────────┘  └──────────┘  └─────────────────┘  │ │
│  └─────────────────────────────────────────────────────┘ │
│                        │                                  │
│  ┌─────────────────────▼──────────────────────────────┐ │
│  │            MongoDB Driver (mongodb crate)            │ │
│  └─────────────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────────────┘
```

### Component Responsibilities

| Component | Responsibility | Runtime |
|-----------|---------------|---------|
| `ChatPanel` | GPUI view, renders `ChatBlock` elements, handles user input | GPUI main thread |
| `AgentLoop` | Orchestrates LLM ↔ tool calls, manages conversation state | Spawned via bridge |
| `Bridge` | Adapts between GPUI async executor and Tokio runtime | Both |
| `Rig Agent` | Sends messages to LLM, handles tool calling protocol | Tokio |
| `Tools` | Execute MongoDB operations, return structured results | Tokio |
| `Safety` | Validates queries before execution, classifies risk tier | Sync (called from tools) |
| `Context` | Samples schemas, builds system prompts | Tokio |

---

## 3. LLM Provider Layer — Rig Framework

### Why Rig

Rig (`rig-core`) is a Rust-native LLM framework that provides:

- **Unified provider interface** across Anthropic, OpenAI, Gemini, Ollama, and 15+ other providers
- **Typed `Tool` trait** with automatic JSON Schema generation from Rust types
- **Streaming support** via `StreamingPrompt` trait
- **Agent builder pattern** for composing system prompts, tools, and model configuration
- **`rig-mongodb`** companion crate for future vector search / RAG features

### Cargo Dependencies

```toml
[dependencies]
rig-core = { version = "0.11", features = ["anthropic", "openai", "gemini", "ollama"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
keyring = { version = "3", features = ["apple-native"] }
mongodb = "3"
futures = "0.3"
```

### Provider Initialization

```rust
use rig::providers::{anthropic, openai, gemini, ollama};
use rig::client::ProviderClient;

pub enum ProviderConfig {
    Anthropic { api_key: String, model: String },
    OpenAI { api_key: String, model: String },
    Gemini { api_key: String, model: String },
    Ollama { base_url: String, model: String },
}

pub fn create_client(config: &ProviderConfig) -> Box<dyn ProviderClient> {
    match config {
        ProviderConfig::Anthropic { api_key, .. } => {
            Box::new(anthropic::Client::new(api_key))
        }
        ProviderConfig::OpenAI { api_key, .. } => {
            Box::new(openai::Client::new(api_key))
        }
        ProviderConfig::Gemini { api_key, .. } => {
            Box::new(gemini::Client::new(api_key))
        }
        ProviderConfig::Ollama { base_url, .. } => {
            Box::new(ollama::Client::from_url(base_url))
        }
    }
}
```

### Recommended Models per Provider

| Provider | Recommended Model | Context | Notes |
|----------|------------------|---------|-------|
| Anthropic | `claude-sonnet-4-6` | 200K | Best balance of speed + quality for tool use |
| OpenAI | `gpt-4.1` | 1M | Structured outputs with `strict: true` |
| Gemini | `gemini-2.5-flash` | 1M | Fast, cost-effective, good tool calling |
| Ollama | `qwen3:32b` | 32K+ | Best local model for tool calling |

### Building the Rig Agent

```rust
use rig::completion::Prompt;
use rig::streaming::StreamingPrompt;

pub fn build_agent(
    client: &impl ProviderClient,
    model: &str,
    system_prompt: &str,
    tools: Vec<impl Into<rig::tool::ToolSet>>,
) -> impl Prompt + StreamingPrompt {
    client
        .agent(model)
        .preamble(system_prompt)
        .max_tokens(4096)
        .tools(tools)
        .build()
}
```

---

## 4. Tokio ↔ GPUI Runtime Bridge

GPUI uses its own async runtime based on macOS Grand Central Dispatch. Rig depends on Tokio via `reqwest`. These two runtimes must be bridged.

### Bridge Implementation

```rust
use std::sync::Arc;
use tokio::runtime::Runtime as TokioRuntime;

/// Holds a Tokio runtime that lives for the app's lifetime.
/// Created once at app startup, shared via Arc.
pub struct AiBridge {
    tokio_rt: Arc<TokioRuntime>,
}

impl AiBridge {
    pub fn new() -> Self {
        let tokio_rt = TokioRuntime::new()
            .expect("Failed to create Tokio runtime for AI bridge");
        Self {
            tokio_rt: Arc::new(tokio_rt),
        }
    }

    /// Run an async closure on the Tokio runtime from GPUI context.
    /// Returns a GPUI-compatible future that can be awaited in cx.spawn().
    pub fn run<F, T>(&self, future: F) -> tokio::task::JoinHandle<T>
    where
        F: std::future::Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        self.tokio_rt.spawn(future)
    }
}
```

### Usage from GPUI

```rust
impl ChatPanel {
    fn send_message(&mut self, cx: &mut Context<Self>) {
        let bridge = self.bridge.clone();
        let agent = self.agent.clone();
        let messages = self.conversation.clone();

        self.is_streaming = true;
        cx.notify();

        self._task = Some(cx.spawn(async move |this, mut cx| {
            // Run Rig calls on Tokio runtime
            let result = bridge.run(async move {
                agent.prompt(&messages.last_user_message()).await
            }).await;

            // Update GPUI state on main thread
            match result {
                Ok(Ok(response)) => {
                    this.update(&mut cx, |this, cx| {
                        this.handle_response(response);
                        this.is_streaming = false;
                        cx.notify();
                    }).ok();
                }
                Ok(Err(e)) => {
                    this.update(&mut cx, |this, cx| {
                        this.handle_error(e);
                        this.is_streaming = false;
                        cx.notify();
                    }).ok();
                }
                Err(join_err) => { /* Tokio task panicked or was cancelled */ }
            }
        }));
    }
}
```

### Cancellation

Current implementation policy:

- Explicit `Stop` cancels the in-flight AI request.
- Switching tabs/sessions or closing the AI panel keeps the request running in background.
- Closing the owning collection tab/session signals cancellation to avoid orphaned streams.

Implementation detail: cancellation uses a per-session shared atomic cancel flag propagated through provider streaming loops.

Original reference pattern (watch channel or `CancellationToken`) remains a valid alternative:

```rust
use tokio_util::sync::CancellationToken;

pub struct StreamingTask {
    cancel: CancellationToken,
}

impl StreamingTask {
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}
```

---

## 5. Agent Loop

The agent loop is the core orchestration layer. It sends user messages to the LLM, processes tool calls, executes tools (with safety checks), feeds results back, and collects `ChatBlock`s for rendering.

### Loop Implementation

```rust
use rig::completion::{Chat, CompletionResponse};

pub struct AgentLoop {
    bridge: Arc<AiBridge>,
    safety: SafetyChecker,
    mongo_ctx: MongoContext,
}

pub struct AgentResult {
    pub blocks: Vec<ChatBlock>,
    pub conversation: Vec<rig::message::Message>,
}

impl AgentLoop {
    pub async fn run(
        &self,
        agent: &impl Chat,
        conversation: &mut Vec<rig::message::Message>,
        on_block: impl Fn(ChatBlock) + Send,  // stream blocks to UI as they're produced
    ) -> Result<AgentResult, AiError> {
        let max_iterations = 10;
        let mut blocks = vec![];

        for _ in 0..max_iterations {
            let response = agent.chat(conversation).await?;

            // Process text content
            if let Some(text) = response.text() {
                let block = ChatBlock::Text { markdown: text.clone() };
                on_block(block.clone());
                blocks.push(block);
            }

            // If no tool calls, conversation is complete
            let tool_calls = response.tool_calls();
            if tool_calls.is_empty() {
                break;
            }

            // Process each tool call
            for tool_call in tool_calls {
                let tool_result = self.execute_tool(&tool_call, &on_block).await?;

                // If tool requires confirmation, pause the loop
                if let ToolExecResult::NeedsConfirmation(confirmation) = &tool_result {
                    let block = ChatBlock::Confirmation(confirmation.clone());
                    on_block(block.clone());
                    blocks.push(block);
                    // Return early — loop resumes after user confirms/rejects
                    return Ok(AgentResult {
                        blocks,
                        conversation: conversation.clone(),
                        // The caller stores pending_action for later execution
                    });
                }

                // Feed tool result back to conversation for next iteration
                conversation.push(tool_result.to_message());
            }
        }

        Ok(AgentResult {
            blocks,
            conversation: conversation.clone(),
        })
    }

    async fn execute_tool(
        &self,
        tool_call: &ToolCall,
        on_block: &impl Fn(ChatBlock),
    ) -> Result<ToolExecResult, AiError> {
        // Safety check before execution
        let safety = self.safety.classify(&tool_call);

        match safety.tier {
            SafetyTier::AutoExecute => {
                let result = self.run_tool(tool_call).await?;
                // Optionally emit a result block (e.g., data table)
                if let Some(block) = result.to_chat_block() {
                    on_block(block);
                }
                Ok(ToolExecResult::Completed(result))
            }
            SafetyTier::ConfirmFirst | SafetyTier::AlwaysConfirm => {
                // Don't execute — return confirmation request
                let preview = self.preview_tool(tool_call).await?;
                Ok(ToolExecResult::NeedsConfirmation(ConfirmationData {
                    tool_call: tool_call.clone(),
                    severity: safety.tier.into(),
                    description: safety.description,
                    affected_count: preview.affected_count,
                    sample_documents: preview.sample_docs,
                }))
            }
            SafetyTier::Blocked => {
                Ok(ToolExecResult::Blocked(safety.reason))
            }
        }
    }
}
```

### Resuming After Confirmation

When the user clicks "Execute" on a confirmation card:

```rust
impl ChatPanel {
    fn on_confirm_action(&mut self, action_id: ActionId, cx: &mut Context<Self>) {
        let pending = self.pending_actions.remove(&action_id);
        if let Some(pending) = pending {
            // Execute the confirmed tool call
            let bridge = self.bridge.clone();
            let agent_loop = self.agent_loop.clone();

            self._task = Some(cx.spawn(async move |this, mut cx| {
                let result = bridge.run(async move {
                    agent_loop.run_tool(&pending.tool_call).await
                }).await;

                this.update(&mut cx, |this, cx| {
                    match result {
                        Ok(Ok(tool_result)) => {
                            // Add result to conversation and optionally continue agent loop
                            this.conversation.push(tool_result.to_message());
                            this.add_block(tool_result.to_chat_block());
                        }
                        Ok(Err(e)) => this.add_block(ChatBlock::Error { .. }),
                        Err(_) => {}
                    }
                    cx.notify();
                }).ok();
            }));
        }
    }

    fn on_reject_action(&mut self, action_id: ActionId, cx: &mut Context<Self>) {
        self.pending_actions.remove(&action_id);
        self.add_block(ChatBlock::Text {
            markdown: "Action cancelled.".into()
        });
        cx.notify();
    }
}
```

---

## 6. Tool Definitions

Each tool implements Rig's `Tool` trait. Tools receive typed arguments from the LLM and return typed outputs that get serialized back to the conversation.

### Tool Trait (from Rig)

```rust
// This is Rig's trait — we implement it, not define it
pub trait Tool: Send + Sync {
    const NAME: &'static str;
    type Error: std::error::Error + Send + Sync;
    type Args: DeserializeOwned + Send;
    type Output: Serialize + Send;

    async fn definition(&self, prompt: String) -> ToolDefinition;
    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error>;
}
```

### Tool: RunFind

```rust
#[derive(Deserialize)]
pub struct RunFindArgs {
    /// The collection to query
    pub collection: String,
    /// MongoDB filter document as JSON string
    pub filter: String,
    /// Optional projection document as JSON string
    pub projection: Option<String>,
    /// Optional sort document as JSON string
    pub sort: Option<String>,
    /// Maximum documents to return (default: 20)
    pub limit: Option<i64>,
}

#[derive(Serialize)]
pub struct FindResult {
    pub documents: Vec<serde_json::Value>,
    pub count: u64,
    pub has_more: bool,
}

#[derive(Deserialize, Serialize)]
pub struct RunFindTool {
    #[serde(skip)]
    pub mongo_ctx: Arc<MongoContext>,
}

impl Tool for RunFindTool {
    const NAME: &'static str = "run_find";
    type Error = MongoToolError;
    type Args = RunFindArgs;
    type Output = FindResult;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "run_find".into(),
            description: "Execute a MongoDB find query and return matching documents. \
                          Use for simple filtering, projection, and sorting. \
                          For grouping, joining, or reshaping data, use run_aggregate instead.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "collection": {
                        "type": "string",
                        "description": "Name of the collection to query"
                    },
                    "filter": {
                        "type": "string",
                        "description": "MongoDB filter as JSON. Use Extended JSON for special types: \
                                        {\"$oid\": \"...\"} for ObjectId, {\"$date\": \"...\"} for dates"
                    },
                    "projection": {
                        "type": "string",
                        "description": "Optional projection as JSON, e.g. {\"name\": 1, \"email\": 1}"
                    },
                    "sort": {
                        "type": "string",
                        "description": "Optional sort as JSON, e.g. {\"createdAt\": -1}"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max documents to return. Default 20. Always set a limit."
                    }
                },
                "required": ["collection", "filter"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let db = self.mongo_ctx.current_database()?;
        let collection = db.collection::<bson::Document>(&args.collection);

        let filter: bson::Document = serde_json::from_str(&args.filter)
            .map_err(|e| MongoToolError::InvalidQuery(format!("Invalid filter JSON: {e}")))?;

        let mut options = mongodb::options::FindOptions::default();
        options.limit = Some(args.limit.unwrap_or(20));

        if let Some(proj) = &args.projection {
            options.projection = Some(serde_json::from_str(proj)?);
        }
        if let Some(sort) = &args.sort {
            options.sort = Some(serde_json::from_str(sort)?);
        }

        let mut cursor = collection.find(filter.clone()).with_options(options).await?;
        let mut documents = vec![];
        while let Some(doc) = cursor.try_next().await? {
            documents.push(serde_json::to_value(&doc)?);
        }

        let total_count = collection.count_documents(filter).await?;
        let limit = args.limit.unwrap_or(20) as u64;

        Ok(FindResult {
            count: documents.len() as u64,
            has_more: total_count > limit,
            documents,
        })
    }
}
```

### Tool: RunAggregate

```rust
#[derive(Deserialize)]
pub struct RunAggregateArgs {
    pub collection: String,
    /// Aggregation pipeline as JSON array string
    pub pipeline: String,
}

#[derive(Serialize)]
pub struct AggregateResult {
    pub documents: Vec<serde_json::Value>,
    pub count: u64,
    pub stages_used: Vec<String>,
}

pub struct RunAggregateTool { pub mongo_ctx: Arc<MongoContext> }

impl Tool for RunAggregateTool {
    const NAME: &'static str = "run_aggregate";
    type Error = MongoToolError;
    type Args = RunAggregateArgs;
    type Output = AggregateResult;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "run_aggregate".into(),
            description: "Execute a MongoDB aggregation pipeline. Use for grouping, joining ($lookup), \
                          unwinding arrays, computing statistics, and reshaping documents. \
                          Always include a $limit stage for unbounded pipelines.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "collection": { "type": "string" },
                    "pipeline": {
                        "type": "string",
                        "description": "Aggregation pipeline as JSON array of stage objects"
                    }
                },
                "required": ["collection", "pipeline"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let db = self.mongo_ctx.current_database()?;
        let collection = db.collection::<bson::Document>(&args.collection);

        let pipeline: Vec<bson::Document> = serde_json::from_str(&args.pipeline)?;

        let stages_used: Vec<String> = pipeline.iter()
            .filter_map(|stage| stage.keys().next().map(|k| k.to_string()))
            .collect();

        let mut cursor = collection.aggregate(pipeline).await?;
        let mut documents = vec![];
        while let Some(doc) = cursor.try_next().await? {
            documents.push(serde_json::to_value(&doc)?);
        }

        Ok(AggregateResult {
            count: documents.len() as u64,
            documents,
            stages_used,
        })
    }
}
```

### Complete Tool Catalog

| Tool Name | Args | Output | Safety Tier | Description |
|-----------|------|--------|-------------|-------------|
| `run_find` | collection, filter, projection, sort, limit | documents[], count | Auto-execute | Execute find queries |
| `run_aggregate` | collection, pipeline | documents[], stages | Auto-execute | Execute aggregation pipelines |
| `count_documents` | collection, filter | count | Auto-execute | Count matching documents |
| `get_schema` | collection, sample_size | fields[], types | Auto-execute | Sample and infer collection schema |
| `list_collections` | (none) | collections[] with stats | Auto-execute | List all collections in current DB |
| `list_databases` | (none) | databases[] with stats | Auto-execute | List all databases |
| `list_indexes` | collection | indexes[] | Auto-execute | List collection indexes |
| `explain_query` | collection, filter/pipeline | explain plan | Auto-execute | Run explain on a query |
| `insert_documents` | collection, documents[] | inserted_ids[] | Confirm first | Insert one or more documents |
| `update_documents` | collection, filter, update, multi | modified_count | Confirm first | Update matching documents |
| `delete_documents` | collection, filter, multi | deleted_count | Always confirm | Delete matching documents |
| `create_index` | collection, keys, options | index_name | Confirm first | Create a new index |
| `drop_index` | collection, index_name | success | Always confirm | Drop an index |

---

## 7. Gen UI — Chat Block System

The LLM does not generate UI. It selects tools and returns structured data. The agent loop converts tool results into typed `ChatBlock` variants. GPUI renders each variant as a native component.

### ChatBlock Enum

```rust
#[derive(Clone, Debug, Serialize)]
pub enum ChatBlock {
    /// Markdown text with optional streaming state
    Text {
        markdown: String,
    },

    /// Syntax-highlighted MongoDB query with action buttons
    QueryPreview {
        query_type: QueryType,         // Find, Aggregate, Update, Delete
        collection: String,
        query_display: String,          // Pretty-printed mongosh syntax
        query_raw: serde_json::Value,   // Parsed query for execution
        explanation: String,            // LLM's explanation of the query
    },

    /// Tabular display of query results
    ResultTable {
        columns: Vec<ColumnDef>,
        rows: Vec<Vec<serde_json::Value>>,
        total_count: u64,
        has_more: bool,
        source_query: Option<String>,   // For "Open in tab" button
    },

    /// Single document displayed as collapsible JSON tree
    DocumentTree {
        document: serde_json::Value,
        highlight_paths: Vec<String>,   // Paths to highlight (e.g., matched fields)
    },

    /// Collection schema visualization
    SchemaOutline {
        collection: String,
        fields: Vec<SchemaField>,
    },

    /// Compact metrics card
    StatsCard {
        title: String,
        metrics: Vec<Metric>,
    },

    /// Chart for aggregation results with numeric data
    Chart {
        chart_type: ChartType,
        title: String,
        labels: Vec<String>,
        datasets: Vec<Dataset>,
    },

    /// Approve/reject card for write operations
    Confirmation {
        action_id: ActionId,
        severity: Severity,             // Info, Warning, Danger
        operation: String,              // "deleteMany", "updateMany", etc.
        description: String,            // Human-readable description
        collection: String,
        query_display: String,
        affected_count: Option<u64>,
        sample_affected: Vec<serde_json::Value>,  // Preview of affected docs
    },

    /// Aggregation pipeline displayed stage by stage
    PipelineView {
        collection: String,
        stages: Vec<PipelineStage>,
    },

    /// Explain plan visualization
    ExplainView {
        winning_plan: String,
        execution_stats: ExecStats,
        recommendations: Vec<String>,
    },

    /// Index recommendation with one-click create
    IndexRecommendation {
        collection: String,
        keys: serde_json::Value,
        reason: String,
        create_action: ActionId,
    },

    /// Diff view for document modifications
    DiffView {
        before: serde_json::Value,
        after: serde_json::Value,
        description: String,
    },

    /// Error with diagnostic info and suggested fixes
    Error {
        code: Option<i32>,
        message: String,
        suggestion: Option<String>,
        retry_action: Option<ActionId>,
    },

    /// Progress for long-running operations
    Progress {
        label: String,
        current: u64,
        total: u64,
    },

    /// Clickable collection reference that opens in a new tab
    CollectionLink {
        database: String,
        collection: String,
    },
}
```

### Supporting Types

```rust
#[derive(Clone, Debug, Serialize)]
pub struct ColumnDef {
    pub key: String,
    pub label: String,
    pub field_type: FieldType,
}

#[derive(Clone, Debug, Serialize)]
pub struct SchemaField {
    pub path: String,
    pub bson_type: String,
    pub frequency: f64,        // 0.0 to 1.0 — percentage of docs containing this field
    pub sample_value: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Metric {
    pub label: String,
    pub value: String,
    pub unit: Option<String>,
    pub trend: Option<Trend>,  // Up, Down, Stable
}

#[derive(Clone, Debug, Serialize)]
pub struct PipelineStage {
    pub operator: String,      // "$match", "$group", etc.
    pub definition: serde_json::Value,
    pub output_preview: Option<Vec<serde_json::Value>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ExecStats {
    pub execution_time_ms: u64,
    pub docs_examined: u64,
    pub docs_returned: u64,
    pub keys_examined: u64,
    pub index_used: Option<String>,
    pub is_collscan: bool,
}

pub type ActionId = uuid::Uuid;

#[derive(Clone, Debug, Serialize)]
pub enum Severity { Info, Warning, Danger }

#[derive(Clone, Debug, Serialize)]
pub enum ChartType { Bar, Line, Pie, HorizontalBar }

#[derive(Clone, Debug, Serialize)]
pub enum QueryType { Find, Aggregate, Count, Update, Delete, Insert }
```

### Tool Result → ChatBlock Mapping

After each tool executes, the agent loop converts the raw tool output into appropriate `ChatBlock`(s):

| Tool | Primary ChatBlock | Condition |
|------|-------------------|-----------|
| `run_find` | `ResultTable` | Always |
| `run_aggregate` | `ResultTable` or `Chart` | `Chart` if all values are numeric and ≤20 rows |
| `count_documents` | `StatsCard` | Always |
| `get_schema` | `SchemaOutline` | Always |
| `list_collections` | `ResultTable` | Always |
| `list_indexes` | `ResultTable` | Always |
| `explain_query` | `ExplainView` + optional `IndexRecommendation` | Recommendation if COLLSCAN detected |
| Write tools | `Confirmation` | Before execution |
| Write tools (confirmed) | `StatsCard` | After execution |

---

## 8. Context Assembly

The quality of LLM-generated queries depends heavily on the context provided. Before each LLM call, the agent assembles a context document that is injected into the system prompt.

### MongoContext Struct

```rust
pub struct MongoContext {
    pub client: mongodb::Client,
    pub current_database: Option<String>,
    pub current_collection: Option<String>,
}

pub struct AssembledContext {
    pub server_version: String,
    pub database_name: String,
    pub collection_name: Option<String>,
    pub schema_sample: Option<SchemaSample>,
    pub indexes: Option<Vec<IndexInfo>>,
    pub collection_stats: Option<CollectionStats>,
    pub current_date: String,
}

pub struct SchemaSample {
    pub type_annotations: String,   // TypeScript-style type definition
    pub sample_documents: Vec<bson::Document>,  // 2-3 representative docs
}
```

### Schema Sampling Strategy

```rust
impl MongoContext {
    pub async fn sample_schema(
        &self,
        collection_name: &str,
        sample_size: usize,
    ) -> Result<SchemaSample, MongoError> {
        let db = self.client.database(&self.current_database.as_ref().unwrap());
        let collection = db.collection::<bson::Document>(collection_name);

        // Sample representative documents
        let pipeline = vec![doc! { "$sample": { "size": sample_size.min(5) as i64 } }];
        let mut cursor = collection.aggregate(pipeline).await?;
        let mut samples = vec![];
        while let Some(doc) = cursor.try_next().await? {
            samples.push(doc);
        }

        // Infer TypeScript-style types from samples
        let type_annotations = infer_typescript_types(&samples);

        // Truncate large values (embeddings, long strings, big arrays)
        let truncated_samples: Vec<bson::Document> = samples.iter()
            .map(|doc| truncate_document(doc, 200))  // max 200 chars per value
            .collect();

        Ok(SchemaSample {
            type_annotations,
            sample_documents: truncated_samples,
        })
    }
}
```

### Type Inference Example

Given sample documents, the `infer_typescript_types` function produces:

```typescript
// Inferred from collection: users
interface UsersDocument {
  _id: ObjectId;
  name: string;
  email: string;
  age: number;                          // integer
  address: {
    street: string;
    city: string;
    zipCode: string;
  };
  tags: string[];                       // present in 85% of documents
  createdAt: Date;
  lastLogin: Date | null;               // null in 12% of documents
  metadata?: Record<string, unknown>;   // present in 45% of documents
}
```

This format is more effective for LLM comprehension than raw JSON Schema because models have been heavily trained on TypeScript definitions.

---

## 9. MongoDB Query Generation — Prompt Engineering

### System Prompt Template

```rust
const SYSTEM_PROMPT_TEMPLATE: &str = r#"
You are a MongoDB expert assistant integrated into OpenMango, a MongoDB client application.
Your role is to help users query, analyze, and manage their MongoDB databases.

## Capabilities
You have access to tools that can:
- Execute find queries and aggregation pipelines
- Analyze collection schemas and indexes
- Explain query performance
- Create and drop indexes
- Insert, update, and delete documents (requires user confirmation)

## Current Context
- **Database:** {database_name}
- **Server version:** {server_version}
- **Current date:** {current_date}

{schema_section}

{index_section}

## Rules for Query Generation

1. **Use Extended JSON for special types:**
   - ObjectId: {"$oid": "507f1f77bcf86cd799439011"}
   - Date: {"$date": "2025-01-15T00:00:00Z"}
   - NumberLong: {"$numberLong": "9223372036854775807"}
   - NumberDecimal: {"$numberDecimal": "123.40"}

2. **Choose the right operation:**
   - Use `run_find` for simple filtering, projection, and sorting
   - Use `run_aggregate` for grouping, $lookup joins, $unwind, computing stats, or reshaping
   - Always include a limit for unbounded queries

3. **Performance awareness:**
   - Check available indexes before writing queries
   - Prefer queries that use existing indexes
   - If a query requires a COLLSCAN on a large collection, suggest creating an index

4. **Multi-step reasoning:**
   - You can call multiple tools in sequence to answer complex questions
   - For example: list_collections → get_schema → run_aggregate
   - Always start by understanding the schema if you haven't seen the collection before

5. **Safety:**
   - Write operations (insert, update, delete) will require user confirmation
   - Never generate queries with empty filters for update/delete operations
   - Always explain what a query will do before executing mutations
"#;
```

### Dynamic Sections

```rust
fn build_schema_section(schema: &SchemaSample) -> String {
    format!(
        "## Collection Schema\n\
         ```typescript\n{}\n```\n\n\
         ### Sample Documents (Extended JSON)\n\
         ```json\n{}\n```",
        schema.type_annotations,
        schema.sample_documents.iter()
            .map(|d| serde_json::to_string_pretty(d).unwrap())
            .collect::<Vec<_>>()
            .join("\n---\n")
    )
}

fn build_index_section(indexes: &[IndexInfo]) -> String {
    if indexes.is_empty() {
        return "## Indexes\nNo indexes found (only _id).".into();
    }
    let mut section = "## Indexes\n".to_string();
    for idx in indexes {
        section.push_str(&format!("- `{}`: {} {}\n",
            idx.name,
            serde_json::to_string(&idx.keys).unwrap(),
            if idx.unique { "(unique)" } else { "" }
        ));
    }
    section
}
```

### Few-Shot Examples (Included in System Prompt)

```
## Examples

User: "Find all orders over $500 from last month"
→ Tool: run_find
  collection: "orders"
  filter: {"total": {"$gt": 500}, "createdAt": {"$gte": {"$date": "2026-01-01T00:00:00Z"}, "$lt": {"$date": "2026-02-01T00:00:00Z"}}}
  sort: {"total": -1}
  limit: 50

User: "Average order value by customer tier"
→ Tool: run_aggregate
  collection: "orders"
  pipeline: [
    {"$group": {"_id": "$customerTier", "avgValue": {"$avg": "$total"}, "count": {"$sum": 1}}},
    {"$sort": {"avgValue": -1}}
  ]

User: "Which collection has the most documents?"
→ Tool: list_collections (first, to get all collections)
→ Then analyze stats from the result to answer
```

---

## 10. Safety and Permission Model

### Four-Tier Classification

```rust
#[derive(Clone, Debug)]
pub enum SafetyTier {
    /// Read-only operations. Execute immediately, show results inline.
    AutoExecute,
    /// Write operations. Show preview, require "Execute" button click.
    ConfirmFirst,
    /// Destructive operations. Show red warning, affected count, require explicit confirm.
    AlwaysConfirm,
    /// Administrative operations. Never allow through chat.
    Blocked,
}

pub struct SafetyChecker;

impl SafetyChecker {
    pub fn classify(&self, tool_call: &ToolCall) -> SafetyClassification {
        match tool_call.name.as_str() {
            // Tier 1: Auto-execute
            "run_find" | "run_aggregate" | "count_documents" |
            "get_schema" | "list_collections" | "list_databases" |
            "list_indexes" | "explain_query" => {
                SafetyClassification {
                    tier: SafetyTier::AutoExecute,
                    description: "Read-only operation".into(),
                    reason: None,
                }
            }

            // Tier 2: Confirm first
            "insert_documents" | "create_index" => {
                SafetyClassification {
                    tier: SafetyTier::ConfirmFirst,
                    description: format!("Write operation: {}", tool_call.name),
                    reason: None,
                }
            }

            // Tier 3: Always confirm (with extra checks)
            "update_documents" | "delete_documents" | "drop_index" => {
                let tier = self.check_destructive_patterns(tool_call);
                SafetyClassification {
                    tier,
                    description: format!("Destructive operation: {}", tool_call.name),
                    reason: None,
                }
            }

            _ => SafetyClassification {
                tier: SafetyTier::Blocked,
                description: "Unknown tool — blocked by default".into(),
                reason: Some("Unrecognized tool name".into()),
            }
        }
    }

    fn check_destructive_patterns(&self, tool_call: &ToolCall) -> SafetyTier {
        // Parse the filter from arguments
        if let Some(filter_str) = tool_call.args.get("filter").and_then(|v| v.as_str()) {
            if let Ok(filter) = serde_json::from_str::<serde_json::Value>(filter_str) {
                // Empty filter on delete/update = affects ALL documents
                if filter.as_object().map_or(true, |o| o.is_empty()) {
                    return SafetyTier::Blocked;
                    // Or: AlwaysConfirm with a very strong warning
                }
                // $where with JavaScript
                if filter.get("$where").is_some() {
                    return SafetyTier::Blocked;
                }
            }
        }
        SafetyTier::AlwaysConfirm
    }
}
```

### Pre-Execution Preview

Before any write operation is confirmed, run a preview:

```rust
pub struct OperationPreview {
    pub affected_count: u64,
    pub sample_docs: Vec<serde_json::Value>,  // First 3 affected documents
    pub execution_plan: Option<String>,        // explain() output summary
}

impl AgentLoop {
    async fn preview_tool(&self, tool_call: &ToolCall) -> Result<OperationPreview, AiError> {
        let collection = self.get_collection_from_args(tool_call)?;
        let filter = self.get_filter_from_args(tool_call)?;

        // Count affected documents
        let count = collection.count_documents(filter.clone()).await?;

        // Sample affected documents
        let samples = collection.find(filter.clone())
            .limit(3)
            .await?
            .try_collect::<Vec<_>>()
            .await?;

        Ok(OperationPreview {
            affected_count: count,
            sample_docs: samples.into_iter()
                .map(|d| serde_json::to_value(d).unwrap())
                .collect(),
            execution_plan: None,
        })
    }
}
```

### Undo Support

For confirmed mutations, capture pre-images for rollback:

```rust
pub struct UndoSnapshot {
    pub id: uuid::Uuid,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub operation: String,
    pub collection: String,
    pub filter: bson::Document,
    pub original_documents: Vec<bson::Document>,
}

// Store in a local SQLite database or in-memory with configurable retention
```

---

## 11. Streaming and UI Integration

### Streaming Text Responses

For text-only responses (no tool calls), stream tokens to the UI as they arrive:

```rust
impl ChatPanel {
    fn stream_response(&mut self, cx: &mut Context<Self>) {
        let bridge = self.bridge.clone();
        let agent = self.agent.clone();
        let prompt = self.input_text.clone();

        self.is_streaming = true;
        self.streaming_block_id = Some(self.add_empty_text_block());
        cx.notify();

        self._stream_task = Some(cx.spawn(async move |this, mut cx| {
            let result = bridge.run(async move {
                let mut stream = agent.stream_prompt(&prompt).await?;
                let mut full_text = String::new();

                while let Some(chunk) = stream.next().await {
                    match chunk {
                        Ok(delta) => {
                            if let Some(text) = delta.text() {
                                full_text.push_str(text);
                                // Clone for the closure
                                let snapshot = full_text.clone();
                                this.update(&mut cx, |this, cx| {
                                    this.update_streaming_text(&snapshot);
                                    cx.notify();
                                }).ok();
                            }
                        }
                        Err(e) => return Err(e.into()),
                    }
                }
                Ok(full_text)
            }).await;

            this.update(&mut cx, |this, cx| {
                this.finalize_streaming();
                this.is_streaming = false;
                cx.notify();
            }).ok();
        }));
    }
}
```

### Token Budget Management

Keep conversation history within the model's context window:

```rust
const MAX_CONTEXT_TOKENS_ESTIMATE: usize = 100_000;  // Conservative for 200K models
const TOKENS_PER_CHAR_ESTIMATE: f64 = 0.3;           // Rough approximation

pub fn trim_conversation(
    messages: &mut Vec<ChatMessage>,
    system_prompt_chars: usize,
) {
    let system_tokens = (system_prompt_chars as f64 * TOKENS_PER_CHAR_ESTIMATE) as usize;
    let available = MAX_CONTEXT_TOKENS_ESTIMATE - system_tokens - 4096; // Reserve for output

    // Always keep: first message (system context) + last 2 exchanges
    // Trim from the middle
    let mut total_chars: usize = messages.iter().map(|m| m.content_len()).sum();
    let total_tokens_est = (total_chars as f64 * TOKENS_PER_CHAR_ESTIMATE) as usize;

    if total_tokens_est <= available {
        return;
    }

    // Remove oldest messages (after system, before last 4)
    while messages.len() > 5 {
        let est = (messages.iter().map(|m| m.content_len()).sum::<usize>() as f64
            * TOKENS_PER_CHAR_ESTIMATE) as usize;
        if est <= available { break; }
        messages.remove(1);  // Remove second message (preserve first = system context)
    }
}
```

---

## 12. API Key Management

Store API keys in macOS Keychain using the `keyring` crate with `apple-native` feature.

```rust
use keyring::Entry;

const SERVICE_NAME: &str = "com.openmango.ai-keys";

pub struct KeyStore;

impl KeyStore {
    pub fn store(provider: &str, api_key: &str) -> Result<(), KeyStoreError> {
        let entry = Entry::new(SERVICE_NAME, provider)?;
        entry.set_password(api_key)?;
        Ok(())
    }

    pub fn get(provider: &str) -> Result<Option<String>, KeyStoreError> {
        let entry = Entry::new(SERVICE_NAME, provider)?;
        match entry.get_password() {
            Ok(key) => Ok(Some(key)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn delete(provider: &str) -> Result<(), KeyStoreError> {
        let entry = Entry::new(SERVICE_NAME, provider)?;
        entry.delete_credential()?;
        Ok(())
    }

    /// Fallback: check environment variables
    pub fn get_with_env_fallback(provider: &str) -> Option<String> {
        Self::get(provider).ok().flatten().or_else(|| {
            let env_key = match provider {
                "anthropic" => "ANTHROPIC_API_KEY",
                "openai" => "OPENAI_API_KEY",
                "gemini" => "GEMINI_API_KEY",
                _ => return None,
            };
            std::env::var(env_key).ok()
        })
    }
}
```

---

## 13. Configuration and Settings

### AI Settings Struct

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiSettings {
    pub enabled: bool,
    pub provider: ProviderType,
    pub model: String,

    // Ollama-specific
    pub ollama_base_url: String,  // default: "http://localhost:11434"

    // Behavior
    pub auto_execute_reads: bool,          // default: true
    pub confirm_writes: bool,              // default: true
    pub max_agent_iterations: usize,       // default: 10
    pub max_results_in_chat: usize,        // default: 50
    pub include_schema_in_context: bool,   // default: true
    pub include_indexes_in_context: bool,  // default: true

    // Privacy
    pub send_sample_documents: bool,       // default: true (can disable for sensitive data)
    pub max_sample_docs: usize,            // default: 3
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ProviderType {
    Anthropic,
    OpenAI,
    Gemini,
    Ollama,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: ProviderType::Anthropic,
            model: "claude-sonnet-4-6".into(),
            ollama_base_url: "http://localhost:11434".into(),
            auto_execute_reads: true,
            confirm_writes: true,
            max_agent_iterations: 10,
            max_results_in_chat: 50,
            include_schema_in_context: true,
            include_indexes_in_context: true,
            send_sample_documents: true,
            max_sample_docs: 3,
        }
    }
}
```

---

## 14. File Structure

```
src/ai/
├── mod.rs                  // Public API: AiFeature, init(), shutdown()
├── bridge.rs               // AiBridge: Tokio ↔ GPUI runtime bridge
├── agent.rs                // AgentLoop: orchestration, tool dispatch, confirmation flow
├── settings.rs             // AiSettings, ProviderType, defaults
├── context.rs              // MongoContext, schema sampling, system prompt assembly
├── safety.rs               // SafetyChecker, SafetyTier, query validation
├── blocks.rs               // ChatBlock enum, supporting types, ActionId
├── keystore.rs             // KeyStore: macOS Keychain + env var fallback
├── convert.rs              // Tool result → ChatBlock conversion logic
├── tools/
│   ├── mod.rs              // Tool registry, MongoToolError
│   ├── query.rs            // RunFindTool, RunAggregateTool, CountDocumentsTool
│   ├── schema.rs           // GetSchemaTool, ListCollectionsTool, ListDatabasesTool
│   ├── index.rs            // ListIndexesTool, CreateIndexTool, DropIndexTool
│   ├── explain.rs          // ExplainQueryTool
│   ├── mutation.rs         // InsertDocumentsTool, UpdateDocumentsTool, DeleteDocumentsTool
│   └── undo.rs             // UndoSnapshot, snapshot/restore logic
└── ui/
    ├── mod.rs              // ChatPanel GPUI view
    ├── message_view.rs     // Renders individual ChatBlock as GPUI element
    ├── input_bar.rs        // Chat input with send button, model selector
    ├── confirmation.rs     // Confirmation card component
    ├── result_table.rs     // Inline data table component
    ├── code_block.rs       // Syntax-highlighted query display
    ├── chart.rs            // Simple bar/line/pie charts
    └── settings_panel.rs   // AI configuration UI
```

---

## 15. Implementation Phases

### Phase 1 — Foundation (Weeks 1-3)

**Goal:** Chat panel with single-provider support, read-only tools, text rendering.

- [x] Set up `rig-core` dependency with Anthropic provider feature
- [x] Implement `AiBridge` (Tokio ↔ GPUI runtime bridge)
- [x] Implement `KeyStore` with macOS Keychain
- [x] Build `ChatPanel` GPUI view with input bar and message list
- [x] Implement `ChatBlock::Text` rendering with markdown
- [x] Implement `ChatBlock::QueryPreview` with syntax highlighting
- [x] Create tools: `run_find`, `run_aggregate`, `count_documents`
- [x] Create tools: `list_collections`, `list_databases`, `get_schema`
- [x] Implement basic agent loop (single iteration, no multi-turn yet)
- [x] Build `MongoContext` with schema sampling
- [x] Write system prompt template with few-shot examples
- [x] AI settings panel in preferences (API key input, model selection)

**Deliverable:** User can open chat, type a natural language query, see the generated MongoDB query, and view results in an inline table.

### Phase 2 — Multi-Turn and Gen UI (Weeks 4-6)

**Goal:** Full agent loop with multi-turn, all Gen UI components, explain/index tools.

- [x] Implement full agent loop with multi-iteration support
- [x] Implement `ChatBlock::ResultTable` with pagination and "Open in tab"
- [x] Implement `ChatBlock::SchemaOutline`
- [x] Implement `ChatBlock::StatsCard`
- [x] Implement `ChatBlock::ExplainView` with performance analysis
- [x] Implement `ChatBlock::IndexRecommendation`
- [x] Create tools: `list_indexes`, `explain_query`
- [x] Token budget management and conversation trimming
- [x] Streaming text responses with incremental rendering
- [x] Add conversation history (persist across tab switches)

**Deliverable:** User can have multi-turn conversations. "Which collection has the most documents?" triggers list_collections → stats analysis → answer. Explain queries show performance insights.

### Phase 3 — Write Operations and Safety (Weeks 7-9)

**Goal:** Write tools with confirmation flow, undo support, full safety model.

- [x] Implement `SafetyChecker` with four-tier classification
- [x] Implement `ChatBlock::Confirmation` with approve/reject buttons
- [x] Create tools: `insert_documents`, `update_documents`, `delete_documents`
- [x] Create tools: `create_index`, `drop_index`
- [x] Pre-execution preview (count affected, show sample docs)
- [x] `ChatBlock::DiffView` for update previews
- [x] `UndoSnapshot` capture and restore
- [x] Empty filter detection and blocking
- [x] `ChatBlock::Progress` for bulk operations

**Deliverable:** User can ask "delete all inactive users older than 2 years" and see a confirmation card showing affected count, sample documents, and approve/reject buttons.

### Phase 4 — Multi-Provider and Polish (Weeks 10-12)

**Goal:** All four providers, charts, keyboard shortcuts, production polish.

- [x] Add OpenAI provider support via Rig
- [x] Add Gemini provider support via Rig
- [x] Add Ollama provider support via Rig (with model detection)
- [x] `ChatBlock::Chart` rendering (bar, line, pie)
- [x] `ChatBlock::CollectionLink` (clickable navigation)
- [x] Keyboard shortcut: `Cmd+L` to focus chat, `Cmd+Enter` to send
- [x] Chat session persistence (save/restore conversations)
- [x] Error handling polish (network errors, rate limits, invalid keys)
- [x] Ollama auto-detection (check if running on localhost:11434)
- [x] Context-aware suggestions (show available collections as autocomplete)

**Deliverable:** Full-featured AI chat with all four providers, interactive charts, and polished UX.

---

## Appendix A — Provider API Reference

### Model Strings

| Provider | Model ID | Context Window | Max Output |
|----------|---------|----------------|------------|
| Anthropic | `claude-sonnet-4-6` | 200K | 64K |
| Anthropic | `claude-haiku-4-5` | 200K | 64K |
| OpenAI | `gpt-4.1` | 1M | 32K |
| OpenAI | `gpt-4.1-mini` | 1M | 32K |
| Gemini | `gemini-2.5-flash` | 1M | 65K |
| Gemini | `gemini-2.5-pro` | 1M | 65K |
| Ollama | `qwen3:32b` | 32K | — |
| Ollama | `llama3.3:70b` | 128K | — |

### Rig Provider Feature Flags

Enable only what you need in `Cargo.toml`:

```toml
rig-core = { version = "0.11", default-features = false, features = [
    "anthropic",
    "openai",
    "gemini",
    "ollama",
] }
```

### Provider-Specific Notes

**Anthropic:** Best tool-calling accuracy (86.7% on MongoDB benchmark). Claude's `strict: true` mode guarantees valid tool call JSON. Streaming emits `input_json_delta` for tool arguments. Rig handles all of this internally.

**OpenAI:** Structured Outputs with `strict: true` guarantees 100% schema adherence via constrained decoding. Requires `additionalProperties: false` on all objects. Rig handles this automatically when building tool definitions.

**Gemini:** Use `mode: ANY` in `functionCallingConfig` to guarantee tool calls when tools are provided. Supports compositional (sequential) function calling natively. Rig's Gemini provider handles the `functionDeclarations` format.

**Ollama:** Tool calling quality varies significantly by model. `qwen3` models (8B+) and `llama3.3` have the best tool-calling reliability. Some models return tool arguments as strings instead of the declared type (e.g., `"2"` instead of `2`) — add defensive parsing in tool implementations. Rig's Ollama provider uses the OpenAI-compatible endpoint by default.

---

## Appendix B — BSON Type Mapping

When converting between LLM-generated Extended JSON and the MongoDB Rust driver's BSON types:

| BSON Type | Extended JSON (Relaxed) | Rust bson Type | LLM Output Format |
|-----------|------------------------|----------------|-------------------|
| ObjectId | `{"$oid": "..."}` | `bson::oid::ObjectId` | `{"$oid": "507f1f77..."}` |
| Date | `{"$date": "..."}` | `bson::DateTime` | `{"$date": "2025-01-15T00:00:00Z"}` |
| Int64 | `{"$numberLong": "..."}` | `i64` | `{"$numberLong": "123456"}` |
| Decimal128 | `{"$numberDecimal": "..."}` | `bson::Decimal128` | `{"$numberDecimal": "19.99"}` |
| Binary | `{"$binary": {...}}` | `bson::Binary` | Rarely needed in queries |
| Regex | `{"$regularExpression": {...}}` | `bson::Regex` | `{"$regex": "pattern", "$options": "i"}` |
| Timestamp | `{"$timestamp": {...}}` | `bson::Timestamp` | Rarely needed |

The `serde_json::from_str` → `bson::Document` conversion handles Extended JSON automatically when using the `bson` crate's serde integration. Instruct the LLM to always use Extended JSON in tool call arguments.

---

*End of specification.*
