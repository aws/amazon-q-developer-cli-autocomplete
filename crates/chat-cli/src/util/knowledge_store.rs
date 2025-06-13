use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{
    Arc,
    LazyLock as Lazy,
};
use std::time::SystemTime;

use eyre::Result;
use semantic_search_client::{
    KnowledgeContext,
    SearchResult,
    SemanticSearchClient,
};
use tokio::sync::Mutex;
use tracing::warn;

// Background operation tracking
#[derive(Debug, Clone)]
pub struct BackgroundOperation {
    pub operation_type: OperationType,
    pub status: OperationStatus,
    pub progress: ProgressInfo,
    pub started_at: SystemTime,
    pub task_handle: Option<tokio::task::AbortHandle>,
}

#[derive(Debug, Clone)]
pub enum OperationType {
    Indexing { name: String },
    Updating { name: String },
}

impl OperationType {
    pub fn display_name(&self) -> String {
        match self {
            OperationType::Indexing { name } => format!("Indexing {}", name),
            OperationType::Updating { name } => format!("Updating {}", name),
        }
    }
}

#[derive(Debug, Clone)]
pub enum OperationStatus {
    Running,
    Completed,
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct ProgressInfo {
    pub current: u64,
    pub total: u64,
    pub message: String,
    pub last_updated: SystemTime,
    pub started_at: SystemTime, // Track when this specific progress phase started
}

impl Default for ProgressInfo {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgressInfo {
    pub fn new() -> Self {
        let now = SystemTime::now();
        Self {
            current: 0,
            total: 0,
            message: "Starting...".to_string(),
            last_updated: now,
            started_at: now,
        }
    }

    pub fn update(&mut self, current: u64, total: u64, message: String) {
        self.current = current;
        self.total = total;
        self.message = message;
        self.last_updated = SystemTime::now();
    }

    /// Calculate estimated time remaining based on current progress
    pub fn calculate_eta(&self) -> Option<std::time::Duration> {
        if self.current == 0 || self.total == 0 || self.current >= self.total {
            return None;
        }

        let elapsed = self.started_at.elapsed().ok()?;
        let progress_ratio = self.current as f64 / self.total as f64;

        if progress_ratio <= 0.0 {
            return None;
        }

        let estimated_total_time = elapsed.as_secs_f64() / progress_ratio;
        let remaining_time = estimated_total_time - elapsed.as_secs_f64();

        if remaining_time > 0.0 {
            Some(std::time::Duration::from_secs_f64(remaining_time))
        } else {
            None
        }
    }
}

// Knowledge store implementation using semantic_search_client
pub struct KnowledgeStore {
    client: Arc<Mutex<SemanticSearchClient>>,
    // Track background operations
    background_operations: Arc<Mutex<HashMap<String, BackgroundOperation>>>,
}

// Configuration constants
const MAX_FILES_LIMIT: u64 = 5000; // Maximum number of files allowed for indexing

impl KnowledgeStore {
    pub(crate) fn new() -> Result<Self> {
        let config = semantic_search_client::SemanticSearchConfig::with_max_files(MAX_FILES_LIMIT as usize);
        match SemanticSearchClient::with_config(SemanticSearchClient::get_default_base_dir(), config) {
            Ok(client) => Ok(Self {
                client: Arc::new(Mutex::new(client)),
                background_operations: Arc::new(Mutex::new(HashMap::new())),
            }),
            Err(e) => Err(eyre::eyre!("Failed to create semantic search client: {}", e)),
        }
    }

    // Create a test instance with an isolated directory
    pub(crate) fn new_test_instance() -> Result<Self> {
        let temp_dir = tempfile::tempdir()?;
        let config = semantic_search_client::SemanticSearchConfig::with_max_files(MAX_FILES_LIMIT as usize);
        match SemanticSearchClient::with_config(temp_dir.path(), config) {
            Ok(client) => Ok(Self {
                client: Arc::new(Mutex::new(client)),
                background_operations: Arc::new(Mutex::new(HashMap::new())),
            }),
            Err(e) => Err(eyre::eyre!("Failed to create test semantic search client: {}", e)),
        }
    }

    // Singleton pattern for knowledge store with test mode support
    pub fn get_instance() -> Arc<Mutex<Self>> {
        static INSTANCE: Lazy<Arc<Mutex<KnowledgeStore>>> = Lazy::new(|| {
            Arc::new(Mutex::new(
                KnowledgeStore::new().expect("Failed to create knowledge store"),
            ))
        });

        // Check if we're running in a test environment
        if cfg!(test) {
            // For tests, create a new isolated instance each time
            Arc::new(Mutex::new(
                KnowledgeStore::new_test_instance().expect("Failed to create test knowledge store"),
            ))
        } else {
            // For normal operation, use the singleton
            INSTANCE.clone()
        }
    }

    /// Generate a unique operation ID
    fn generate_operation_id() -> String {
        use std::time::{
            SystemTime,
            UNIX_EPOCH,
        };
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();
        format!("op_{}", timestamp)
    }

    /// Get status of all background operations
    pub async fn get_status(&self) -> Result<String, String> {
        let mut operations = self.background_operations.lock().await;

        // Clean up old completed operations (older than 2 minutes)
        let now = SystemTime::now();
        let cleanup_threshold = std::time::Duration::from_secs(120);

        operations.retain(|_, op| {
            match &op.status {
                OperationStatus::Completed | OperationStatus::Failed(_) | OperationStatus::Cancelled => {
                    // Keep if it's recent, remove if it's old
                    now.duration_since(op.started_at).unwrap_or_default() < cleanup_threshold
                },
                OperationStatus::Running => true, // Always keep running operations
            }
        });

        if operations.is_empty() {
            return Ok("No background operations running.".to_string());
        }

        let mut status_report = String::from("📚 Knowledge Operations Status:\n");
        status_report.push_str(&format!("{}\n", "━".repeat(80)));
        for (id, op) in operations.iter() {
            let duration = op.started_at.elapsed().unwrap_or_default();

            // Choose icon and format based on status
            let (icon, progress_display, status_text) = match &op.status {
                OperationStatus::Running => {
                    let progress_bar = create_progress_bar_snapshot(&op.progress);
                    ("🔄", progress_bar, "Running".to_string())
                },
                OperationStatus::Completed => {
                    let progress_bar = "[██████████████████████████████] 100%".to_string(); // 30 chars to match
                    ("✅", progress_bar, "Completed".to_string())
                },
                OperationStatus::Failed(error) => {
                    let progress_bar = "[░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░] Failed".to_string(); // 30 chars
                    ("❌", progress_bar, format!("Failed: {}", error))
                },
                OperationStatus::Cancelled => {
                    let progress_bar = "[░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░] Cancelled".to_string(); // 30 chars
                    ("🛑", progress_bar, "Cancelled".to_string())
                },
            };

            // Format duration - only show for running operations with better styling
            let duration_text = match &op.status {
                OperationStatus::Running => format!("   ⏱️  Duration: {:?}\n", duration),
                _ => String::new(), // No duration for completed/failed/cancelled operations
            };

            // Format the message - only show for running operations with better styling
            let message_text = match &op.status {
                OperationStatus::Running => format!("   📝 {}\n", op.progress.message),
                _ => String::new(), // No progress message for completed operations
            };

            status_report.push_str(&format!(
                "{} {} ({})\n   {}\n{}{}   📊 Status: {}\n",
                icon,
                op.operation_type.display_name(),
                &id[..8.min(id.len())], // Short ID
                progress_display,
                message_text,
                duration_text,
                status_text
            ));
        }

        Ok(status_report)
    }

    /// Cancel a background operation
    pub async fn cancel_operation(&mut self, operation_id: &str) -> Result<String, String> {
        let mut operations = self.background_operations.lock().await;

        if operation_id == "all" {
            let mut cancelled_count = 0;
            for (_, operation) in operations.iter_mut() {
                if matches!(operation.status, OperationStatus::Running) {
                    if let Some(handle) = &operation.task_handle {
                        handle.abort();
                    }
                    operation.status = OperationStatus::Cancelled;
                    cancelled_count += 1;
                }
            }
            Ok(format!("Cancelled {} background operations", cancelled_count))
        } else if let Some(operation) = operations.get_mut(operation_id) {
            if let Some(handle) = &operation.task_handle {
                handle.abort();
            }
            operation.status = OperationStatus::Cancelled;
            Ok(format!("Cancelled operation: {}", operation_id))
        } else {
            Err(format!("Operation not found: {}", operation_id))
        }
    }

    pub async fn add(&mut self, name: &str, value: &str) -> Result<String, String> {
        // This is now fire-and-forget - start background processing and return immediately
        self.add_background(name, value).await
    }

    /// Start background indexing operation (fire-and-forget)
    pub async fn add_background(&mut self, name: &str, value: &str) -> Result<String, String> {
        self.add_background_with_type(name, value, OperationType::Indexing { name: name.to_string() })
            .await
    }

    /// Start background operation with specified type (fire-and-forget)
    async fn add_background_with_type(
        &mut self,
        name: &str,
        value: &str,
        operation_type: OperationType,
    ) -> Result<String, String> {
        let path = PathBuf::from(value);

        if path.exists() {
            let operation_id = Self::generate_operation_id();

            // Create the background operation entry
            let operation = BackgroundOperation {
                operation_type: operation_type.clone(),
                status: OperationStatus::Running,
                progress: ProgressInfo::new(),
                started_at: SystemTime::now(),
                task_handle: None, // Will be set after spawning
            };

            // Store the operation
            self.background_operations
                .lock()
                .await
                .insert(operation_id.clone(), operation);

            // Start the background indexing task
            let client = self.client.clone();
            let background_operations = self.background_operations.clone();
            let path_clone = path.clone();
            let name_clone = name.to_string();
            let operation_id_clone = operation_id.clone();

            let task = tokio::task::spawn(async move {
                // Create progress callback that updates the stored operation
                let progress_callback = {
                    let background_operations = background_operations.clone();
                    let operation_id = operation_id_clone.clone();

                    move |status: semantic_search_client::types::ProgressStatus| {
                        let background_operations = background_operations.clone();
                        let operation_id = operation_id.clone();

                        // Spawn a task to update progress (non-blocking)
                        tokio::spawn(async move {
                            if let Ok(mut operations) = background_operations.try_lock() {
                                if let Some(operation) = operations.get_mut(&operation_id) {
                                    match status {
                                        semantic_search_client::types::ProgressStatus::CountingFiles => {
                                            operation.progress.update(0, 0, "Counting files...".to_string());
                                        },
                                        semantic_search_client::types::ProgressStatus::StartingIndexing(total) => {
                                            operation.progress.update(
                                                0,
                                                total as u64,
                                                format!("Starting indexing ({} files)", total),
                                            );
                                            // Reset start time for ETA calculation when we know the total
                                            operation.progress.started_at = SystemTime::now();
                                        },
                                        semantic_search_client::types::ProgressStatus::Indexing(current, total) => {
                                            operation.progress.update(
                                                current as u64,
                                                total as u64,
                                                format!("Indexing files ({}/{})", current, total),
                                            );
                                        },
                                        semantic_search_client::types::ProgressStatus::CreatingSemanticContext => {
                                            operation
                                                .progress
                                                .update(0, 0, "Creating semantic context...".to_string());
                                        },
                                        semantic_search_client::types::ProgressStatus::GeneratingEmbeddings(
                                            current,
                                            total,
                                        ) => {
                                            operation.progress.update(
                                                current as u64,
                                                total as u64,
                                                format!("Generating embeddings ({}/{})", current, total),
                                            );
                                        },
                                        semantic_search_client::types::ProgressStatus::BuildingIndex => {
                                            operation.progress.update(0, 0, "Building vector index...".to_string());
                                        },
                                        semantic_search_client::types::ProgressStatus::Finalizing => {
                                            operation.progress.update(0, 0, "Finalizing index...".to_string());
                                        },
                                        semantic_search_client::types::ProgressStatus::Complete => {
                                            operation.progress.update(
                                                operation.progress.total,
                                                operation.progress.total,
                                                "Indexing complete!".to_string(),
                                            );
                                            operation.status = OperationStatus::Completed;
                                        },
                                    }
                                }
                            }
                        });
                    }
                };

                // Run the actual indexing in a blocking task
                let result = tokio::task::spawn_blocking(move || {
                    let mut client_guard = client.blocking_lock();
                    client_guard.add_context_from_path(
                        path_clone,
                        &name_clone,
                        &format!("Knowledge context for {}", name_clone),
                        true,
                        Some(progress_callback),
                    )
                })
                .await;

                // Update final status
                if let Ok(mut operations) = background_operations.try_lock() {
                    if let Some(operation) = operations.get_mut(&operation_id_clone) {
                        match result {
                            Ok(Ok(_)) => {
                                operation.status = OperationStatus::Completed;
                                operation.progress.message = "Successfully completed indexing".to_string();
                            },
                            Ok(Err(ref e)) => {
                                operation.status = OperationStatus::Failed(e.to_string());
                                operation.progress.message = format!("Failed: {}", e);
                            },
                            Err(ref join_error) => {
                                if join_error.is_cancelled() {
                                    operation.status = OperationStatus::Cancelled;
                                    operation.progress.message = "Operation was cancelled".to_string();
                                } else {
                                    operation.status = OperationStatus::Failed(format!("Task failed: {}", join_error));
                                    operation.progress.message = format!("Task failed: {}", join_error);
                                }
                            },
                        }
                    }
                }

                result
            });

            // Store the task handle so we can cancel it later
            let abort_handle = task.abort_handle();
            if let Ok(mut operations) = self.background_operations.try_lock() {
                if let Some(operation) = operations.get_mut(&operation_id) {
                    operation.task_handle = Some(abort_handle);
                }
            }

            let action_verb = match &operation_type {
                OperationType::Indexing { .. } => "indexing",
                OperationType::Updating { .. } => "updating",
            };

            Ok(format!(
                "🚀 Started {} '{}' in background.\n📊 Use '/knowledge status' to check progress.\n🆔 Operation ID: {}",
                action_verb,
                name,
                &operation_id[..8] // Show short ID
            ))
        } else {
            // Handle text content (this is quick, so we can do it synchronously)
            let preview: String = value.chars().take(40).collect();
            let mut client_guard = self.client.lock().await;
            client_guard
                .add_context_from_text(value, name, &format!("Text knowledge {}...", preview), true)
                .map_err(|e| e.to_string())
                .map(|_| format!("✅ Added text content '{}' to knowledge base", name))
        }
    }

    pub async fn update_context_by_id(&mut self, context_id: &str, path_str: &str) -> Result<String, String> {
        // First, check if the context exists
        let client_guard = self.client.lock().await;
        let contexts = client_guard.get_contexts();
        let context = contexts.iter().find(|c| c.id == context_id);
        drop(client_guard); // Release the lock as soon as possible

        if context.is_none() {
            return Err(format!("Context with ID '{}' not found", context_id));
        }

        let context = context.unwrap();
        let path = PathBuf::from(path_str);

        if !path.exists() {
            return Err(format!("Path '{}' does not exist", path_str));
        }

        // Remove the existing context first
        let mut client_guard = self.client.lock().await;
        if let Err(e) = client_guard.remove_context_by_id(context_id, true) {
            return Err(format!("Failed to remove existing context: {}", e));
        }
        drop(client_guard);

        // Now use the background pattern with "Updating" operation type
        // This will create a background operation with progress tracking
        self.add_background_with_type(&context.name, path_str, OperationType::Updating {
            name: context.name.clone(),
        })
        .await
    }

    pub async fn update_context_by_name(&mut self, name: &str, path_str: &str) -> Result<String, String> {
        // Find the context ID by name
        let client_guard = self.client.lock().await;
        let contexts = client_guard.get_contexts();
        let context = contexts.iter().find(|c| c.name == name);
        drop(client_guard); // Release the lock

        if let Some(context) = context {
            self.update_context_by_id(&context.id, path_str).await
        } else {
            Err(format!("Context with name '{}' not found", name))
        }
    }

    pub async fn remove_by_id(&mut self, id: &str) -> Result<(), String> {
        let mut client_guard = self.client.lock().await;
        client_guard.remove_context_by_id(id, true).map_err(|e| e.to_string())
    }

    pub async fn remove_by_name(&mut self, name: &str) -> Result<(), String> {
        let mut client_guard = self.client.lock().await;
        client_guard
            .remove_context_by_name(name, true)
            .map_err(|e| e.to_string())
    }

    pub async fn remove_by_path(&mut self, path: &str) -> Result<(), String> {
        let mut client_guard = self.client.lock().await;
        client_guard
            .remove_context_by_path(path, true)
            .map_err(|e| e.to_string())
    }

    pub async fn update_by_path(&mut self, path_str: &str) -> Result<String, String> {
        // Find contexts that might match this path
        let client_guard = self.client.lock().await;
        let contexts = client_guard.get_contexts();
        let matching_context = contexts.iter().find(|c| {
            if let Some(source_path) = &c.source_path {
                source_path == path_str
            } else {
                false
            }
        });
        drop(client_guard); // Release the lock

        if let Some(context) = matching_context {
            // Found a matching context, update it
            self.update_context_by_id(&context.id, path_str).await
        } else {
            // No matching context found
            Err(format!("No context found with path '{}'", path_str))
        }
    }

    pub async fn clear(&mut self) -> Result<usize, String> {
        let client_guard = self.client.lock().await;
        let contexts = client_guard.get_contexts();
        drop(client_guard); // Release the lock before the potentially long operation

        // Track progress
        let mut removed = 0;

        // Hold the lock for the entire operation since we're doing bulk operations
        let mut client_guard = self.client.lock().await;

        for context in contexts.iter() {
            if let Err(e) = client_guard.remove_context_by_id(&context.id, true) {
                warn!("Failed to remove context {}: {}", context.id, e);
            } else {
                removed += 1;
            }
        }

        drop(client_guard);
        Ok(removed)
    }

    pub async fn search(&self, query: &str, context_id: Option<&str>) -> Result<Vec<SearchResult>, String> {
        let client_guard = self.client.lock().await;
        if let Some(id) = context_id {
            client_guard.search_context(id, query, None).map_err(|e| e.to_string())
        } else {
            let results = client_guard.search_all(query, None).map_err(|e| e.to_string())?;

            // Flatten results from all contexts
            let mut flattened = Vec::new();
            for (_, context_results) in results {
                flattened.extend(context_results);
            }

            // Sort by distance (lower is better)
            flattened.sort_by(|a, b| {
                let a_dist = a.distance;
                let b_dist = b.distance;
                a_dist.partial_cmp(&b_dist).unwrap_or(std::cmp::Ordering::Equal)
            });

            Ok(flattened)
        }
    }

    pub async fn get_all(&self) -> Result<Vec<KnowledgeContext>, String> {
        let client_guard = self.client.lock().await;
        Ok(client_guard.get_contexts())
    }
}

/// Create a visual progress bar snapshot with ETA
fn create_progress_bar_snapshot(progress: &ProgressInfo) -> String {
    let percentage = if progress.total > 0 {
        (progress.current * 100) / progress.total
    } else {
        0
    };

    let bar_length = 30; // Longer bar for better visual impact
    let filled = (percentage * bar_length) / 100;
    let empty = bar_length - filled;

    // Use different characters for a more modern look
    let filled_char = "█";
    let partial_char = "▓"; // Add a partial character for smoother transitions
    let empty_char = "░";

    // Add a partial character at the boundary for smoother appearance
    let (filled_str, empty_str) = if filled < bar_length && percentage > 0 {
        let partial_filled = filled.saturating_sub(1);
        (
            format!("{}{}", filled_char.repeat(partial_filled as usize), partial_char),
            empty_char.repeat(empty.saturating_sub(1) as usize),
        )
    } else {
        (filled_char.repeat(filled as usize), empty_char.repeat(empty as usize))
    };

    // Calculate ETA if possible
    let eta_text = if let Some(eta) = progress.calculate_eta() {
        format!(" (ETA: {})", format_duration(eta))
    } else {
        String::new()
    };

    // Show actual progress for running operations
    if progress.total > 0 {
        format!(
            "[{}{}] {}% ({}/{}{})",
            filled_str, empty_str, percentage, progress.current, progress.total, eta_text
        )
    } else {
        // For operations without specific progress counts
        format!("[{}{}] {}%{}", filled_str, empty_str, percentage, eta_text)
    }
}

/// Format duration in a human-readable way
fn format_duration(duration: std::time::Duration) -> String {
    let total_seconds = duration.as_secs();

    if total_seconds < 60 {
        format!("{}s", total_seconds)
    } else if total_seconds < 3600 {
        let minutes = total_seconds / 60;
        let seconds = total_seconds % 60;
        if seconds == 0 {
            format!("{}m", minutes)
        } else {
            format!("{}m {}s", minutes, seconds)
        }
    } else {
        let hours = total_seconds / 3600;
        let minutes = (total_seconds % 3600) / 60;
        if minutes == 0 {
            format!("{}h", hours)
        } else {
            format!("{}h {}m", hours, minutes)
        }
    }
}
