use anyhow::{anyhow, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::fs;

/// Configuration for Cloudflare D1 database connection
#[derive(Debug, Clone)]
pub struct D1Config {
    pub account_id: String,
    pub database_id: String,
    pub api_token: String,
}

impl D1Config {
    /// Create D1Config from environment variables
    pub fn from_env() -> Result<Self> {
        let account_id = std::env::var("CLOUDFLARE_ACCOUNT_ID")
            .map_err(|_| anyhow!("CLOUDFLARE_ACCOUNT_ID environment variable not set"))?;
        let database_id = std::env::var("CLOUDFLARE_D1_DATABASE_ID")
            .map_err(|_| anyhow!("CLOUDFLARE_D1_DATABASE_ID environment variable not set"))?;
        let api_token = std::env::var("CLOUDFLARE_API_TOKEN")
            .map_err(|_| anyhow!("CLOUDFLARE_API_TOKEN environment variable not set"))?;

        Ok(Self {
            account_id,
            database_id,
            api_token,
        })
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct D1QueryResponse {
    pub success: bool,
    pub meta: D1QueryMeta,
    pub results: Option<Vec<Value>>,
}

#[derive(Debug, Deserialize)]
struct CloudflareApiResponse {
    pub success: bool,
    pub result: Option<Vec<D1QueryResponse>>,
    pub errors: Option<Vec<Value>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct D1QueryMeta {
    pub duration: f64,
    pub rows_read: Option<u64>,
    pub rows_written: Option<u64>,
    pub last_row_id: Option<i64>,
    pub changed_db: Option<bool>,
    pub changes: Option<u64>,
    pub size_after: Option<u64>,
}

/// D1 Database Client for Cloudflare D1 HTTP API
pub struct D1Client {
    config: D1Config,
    client: Client,
    base_url: String,
}

impl D1Client {
    /// Create a new D1 client
    pub fn new(config: D1Config) -> Self {
        let client = Client::new();
        let base_url = format!(
            "https://api.cloudflare.com/client/v4/accounts/{}/d1/database/{}",
            config.account_id, config.database_id
        );

        Self {
            config,
            client,
            base_url,
        }
    }

    pub async fn query(&self, sql: &str, params: Option<Vec<Value>>) -> Result<D1QueryResponse> {
        let url = format!("{}/query", self.base_url);
        let body = serde_json::to_string(&json!({"sql": sql, "params": params.unwrap_or_default()}))?;

        let response = self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_token))
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await?;

        let status = response.status();
        let text = response.text().await?;
        
        // Log the raw response for debugging
        if !status.is_success() {
            return Err(anyhow!("HTTP error {}: {}", status, text));
        }
        
        let api_response: CloudflareApiResponse = serde_json::from_str(&text)
            .map_err(|e| anyhow!("Failed to parse API response: {} (response: {})", e, text))?;
        
        if !api_response.success {
            let error_msg = if let Some(errors) = &api_response.errors {
                format!("D1 API error: {:?}", errors)
            } else {
                format!("D1 API error (response: {})", text)
            };
            return Err(anyhow!(error_msg));
        }

        api_response.result
            .and_then(|mut results| results.pop())
            .ok_or_else(|| anyhow!("No result returned"))
    }

    pub async fn execute(&self, sql: &str, params: Option<Vec<Value>>) -> Result<()> {
        self.query(sql, params).await?;
        Ok(())
    }

    pub async fn run_migration_file(&self, file_path: &str) -> Result<()> {
        let sql = fs::read_to_string(file_path).await?;
        let statements: Vec<&str> = sql.split(';')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty() && !s.starts_with("--"))
            .filter(|s| !s.starts_with("PRAGMA")) // Skip PRAGMA statements - D1 may not support them
            .collect();
        
        for (i, statement) in statements.iter().enumerate() {
            if let Err(e) = self.execute(statement, None).await {
                return Err(anyhow!(
                    "Failed to execute migration statement {} of {}: {}\nStatement: {}",
                    i + 1,
                    statements.len(),
                    e,
                    statement
                ));
            }
        }
        Ok(())
    }

    pub async fn initialize(&self, file_path: Option<&str>) -> Result<()> {
        let path = file_path.unwrap_or("migrations/vane.sql");
        self.run_migration_file(path).await
    }
}

// ============================================================================
// Transaction Lifecycle Operations
// ============================================================================

/// Transaction lifecycle record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxLifecycle {
    pub extended_multi_id: String,
    pub sender_address: String,
    pub receiver_address: String,
    pub tx_json: String,
    pub receiver_confirmed: bool,
    pub reverted: bool,
    pub completed: bool,
}

impl D1Client {
    /// Insert or update a transaction lifecycle record
    pub async fn upsert_tx_lifecycle(&self, tx: &TxLifecycle) -> Result<()> {
        let sql = r#"
            INSERT INTO tx_lifecycle (
                extended_multi_id,
                sender_address,
                receiver_address,
                tx_json,
                receiver_confirmed,
                reverted,
                completed
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(extended_multi_id) DO UPDATE SET
                sender_address = excluded.sender_address,
                receiver_address = excluded.receiver_address,
                tx_json = excluded.tx_json,
                receiver_confirmed = excluded.receiver_confirmed,
                reverted = excluded.reverted,
                completed = excluded.completed
        "#;

        let params = vec![
            json!(tx.extended_multi_id),
            json!(tx.sender_address),
            json!(tx.receiver_address),
            json!(tx.tx_json),
            json!(if tx.receiver_confirmed { 1 } else { 0 }),
            json!(if tx.reverted { 1 } else { 0 }),
            json!(if tx.completed { 1 } else { 0 }),
        ];

        self.execute(sql, Some(params)).await?;
        Ok(())
    }

    /// Get a transaction lifecycle record by extended_multi_id
    pub async fn get_tx_lifecycle(&self, extended_multi_id: &str) -> Result<Option<TxLifecycle>> {
        let sql = "SELECT * FROM tx_lifecycle WHERE extended_multi_id = ?1";
        let params = vec![json!(extended_multi_id)];

        let response = self.query(sql, Some(params)).await?;
        
        match response.results {
            Some(results) if !results.is_empty() => {
                let row = &results[0];
                Ok(Some(TxLifecycle {
                    extended_multi_id: row["extended_multi_id"]
                        .as_str()
                        .ok_or_else(|| anyhow!("Missing extended_multi_id"))?
                        .to_string(),
                    sender_address: row["sender_address"]
                        .as_str()
                        .ok_or_else(|| anyhow!("Missing sender_address"))?
                        .to_string(),
                    receiver_address: row["receiver_address"]
                        .as_str()
                        .ok_or_else(|| anyhow!("Missing receiver_address"))?
                        .to_string(),
                    tx_json: row["tx_json"]
                        .as_str()
                        .ok_or_else(|| anyhow!("Missing tx_json"))?
                        .to_string(),
                    receiver_confirmed: row["receiver_confirmed"]
                        .as_u64()
                        .unwrap_or(0) == 1,
                    reverted: row["reverted"].as_u64().unwrap_or(0) == 1,
                    completed: row["completed"].as_u64().unwrap_or(0) == 1,
                }))
            }
            _ => Ok(None),
        }
    }

    /// Get transaction IDs by sender address (internal helper)
    pub(crate) async fn get_tx_ids_by_sender(&self, sender_address: &str) -> Result<Vec<String>> {
        let sql = "SELECT extended_multi_id FROM v_tx_ids_by_sender WHERE sender_address = ?1";
        let params = vec![json!(sender_address)];

        let response = self.query(sql, Some(params)).await?;
        
        match response.results {
            Some(results) => {
                let ids: Result<Vec<String>> = results
                    .iter()
                    .map(|row| {
                        row["extended_multi_id"]
                            .as_str()
                            .ok_or_else(|| anyhow!("Missing extended_multi_id in result"))
                            .map(|s| s.to_string())
                    })
                    .collect();
                ids
            }
            None => Ok(vec![]),
        }
    }

    /// Get transaction IDs by receiver address (private helper)
    async fn get_tx_ids_by_receiver(&self, receiver_address: &str) -> Result<Vec<String>> {
        let sql = "SELECT extended_multi_id FROM v_tx_ids_by_receiver WHERE receiver_address = ?1";
        let params = vec![json!(receiver_address)];

        let response = self.query(sql, Some(params)).await?;
        
        match response.results {
            Some(results) => {
                let ids: Result<Vec<String>> = results
                    .iter()
                    .map(|row| {
                        row["extended_multi_id"]
                            .as_str()
                            .ok_or_else(|| anyhow!("Missing extended_multi_id in result"))
                            .map(|s| s.to_string())
                    })
                    .collect();
                ids
            }
            None => Ok(vec![]),
        }
    }

    /// Get transaction IDs by sender and receiver pair (private helper)
    async fn get_tx_ids_by_pair(
        &self,
        sender_address: &str,
        receiver_address: &str,
    ) -> Result<Vec<String>> {
        let sql = "SELECT extended_multi_id FROM v_tx_ids_by_pair WHERE sender_address = ?1 AND receiver_address = ?2";
        let params = vec![json!(sender_address), json!(receiver_address)];

        let response = self.query(sql, Some(params)).await?;
        
        match response.results {
            Some(results) => {
                let ids: Result<Vec<String>> = results
                    .iter()
                    .map(|row| {
                        row["extended_multi_id"]
                            .as_str()
                            .ok_or_else(|| anyhow!("Missing extended_multi_id in result"))
                            .map(|s| s.to_string())
                    })
                    .collect();
                ids
            }
            None => Ok(vec![]),
        }
    }

    /// Get transaction JSON data by sender address
    /// Returns a list of tx_json strings for all transactions from the sender
    pub async fn get_tx_json_by_sender(&self, sender_address: &str) -> Result<Vec<String>> {
        let ids = self.get_tx_ids_by_sender(sender_address).await?;
        let mut tx_jsons = Vec::new();
        
        for id in ids {
            if let Some(tx) = self.get_tx_lifecycle(&id).await? {
                tx_jsons.push(tx.tx_json);
            }
        }
        
        Ok(tx_jsons)
    }

    /// Get transaction JSON data by receiver address
    /// Returns a list of tx_json strings for all transactions to the receiver
    pub async fn get_tx_json_by_receiver(&self, receiver_address: &str) -> Result<Vec<String>> {
        let ids = self.get_tx_ids_by_receiver(receiver_address).await?;
        let mut tx_jsons = Vec::new();
        
        for id in ids {
            if let Some(tx) = self.get_tx_lifecycle(&id).await? {
                tx_jsons.push(tx.tx_json);
            }
        }
        
        Ok(tx_jsons)
    }

    /// Get transaction JSON data by sender and receiver pair
    /// Returns a list of tx_json strings for transactions between the sender and receiver
    pub async fn get_tx_json_by_pair(
        &self,
        sender_address: &str,
        receiver_address: &str,
    ) -> Result<Vec<String>> {
        let ids = self.get_tx_ids_by_pair(sender_address, receiver_address).await?;
        let mut tx_jsons = Vec::new();
        
        for id in ids {
            if let Some(tx) = self.get_tx_lifecycle(&id).await? {
                tx_jsons.push(tx.tx_json);
            }
        }
        
        Ok(tx_jsons)
    }

    /// Update transaction completion status
    pub async fn update_tx_completed(&self, extended_multi_id: &str, completed: bool) -> Result<()> {
        let sql = "UPDATE tx_lifecycle SET completed = ?1 WHERE extended_multi_id = ?2";
        let params = vec![json!(if completed { 1 } else { 0 }), json!(extended_multi_id)];

        self.execute(sql, Some(params)).await?;
        Ok(())
    }

    /// Update transaction reverted status
    pub async fn update_tx_reverted(&self, extended_multi_id: &str, reverted: bool) -> Result<()> {
        let sql = "UPDATE tx_lifecycle SET reverted = ?1 WHERE extended_multi_id = ?2";
        let params = vec![json!(if reverted { 1 } else { 0 }), json!(extended_multi_id)];

        self.execute(sql, Some(params)).await?;
        Ok(())
    }

    /// Update receiver confirmed status
    pub async fn update_receiver_confirmed(&self, extended_multi_id: &str, confirmed: bool) -> Result<()> {
        let sql = "UPDATE tx_lifecycle SET receiver_confirmed = ?1 WHERE extended_multi_id = ?2";
        let params = vec![json!(if confirmed { 1 } else { 0 }), json!(extended_multi_id)];

        self.execute(sql, Some(params)).await?;
        Ok(())
    }

    /// Get full transaction lifecycle records for sender and receiver pair
    pub async fn get_tx_lifecycle_by_pair(
        &self,
        sender_address: &str,
        receiver_address: &str,
    ) -> Result<Vec<TxLifecycle>> {
        let sql = r#"
            SELECT extended_multi_id, sender_address, receiver_address, tx_json, 
                   receiver_confirmed, reverted, completed
            FROM tx_lifecycle
            WHERE sender_address = ?1 AND receiver_address = ?2
            ORDER BY rowid DESC
        "#;
        let params = vec![json!(sender_address), json!(receiver_address)];

        let response = self.query(sql, Some(params)).await?;
        
        match response.results {
            Some(results) => {
                let mut txs = Vec::new();
                for row in results {
                    txs.push(TxLifecycle {
                        extended_multi_id: row["extended_multi_id"]
                            .as_str()
                            .ok_or_else(|| anyhow!("Missing extended_multi_id"))?
                            .to_string(),
                        sender_address: row["sender_address"]
                            .as_str()
                            .ok_or_else(|| anyhow!("Missing sender_address"))?
                            .to_string(),
                        receiver_address: row["receiver_address"]
                            .as_str()
                            .ok_or_else(|| anyhow!("Missing receiver_address"))?
                            .to_string(),
                        tx_json: row["tx_json"]
                            .as_str()
                            .ok_or_else(|| anyhow!("Missing tx_json"))?
                            .to_string(),
                        receiver_confirmed: row["receiver_confirmed"]
                            .as_u64()
                            .unwrap_or(0) == 1,
                        reverted: row["reverted"].as_u64().unwrap_or(0) == 1,
                        completed: row["completed"].as_u64().unwrap_or(0) == 1,
                    });
                }
                Ok(txs)
            }
            None => Ok(vec![]),
        }
    }

    /// Get all transaction lifecycle records with specific filters (for metrics computation)
    pub async fn get_tx_lifecycle_filtered(
        &self,
        completed: Option<bool>,
        reverted: Option<bool>,
        receiver_confirmed: Option<bool>,
    ) -> Result<Vec<TxLifecycle>> {
        let mut sql = "SELECT extended_multi_id, sender_address, receiver_address, tx_json, receiver_confirmed, reverted, completed FROM tx_lifecycle WHERE 1=1".to_string();
        let mut params = Vec::new();
        let mut param_idx = 1;

        if let Some(c) = completed {
            sql.push_str(&format!(" AND completed = ?{}", param_idx));
            params.push(json!(if c { 1 } else { 0 }));
            param_idx += 1;
        }
        if let Some(r) = reverted {
            sql.push_str(&format!(" AND reverted = ?{}", param_idx));
            params.push(json!(if r { 1 } else { 0 }));
            param_idx += 1;
        }
        if let Some(rc) = receiver_confirmed {
            sql.push_str(&format!(" AND receiver_confirmed = ?{}", param_idx));
            params.push(json!(if rc { 1 } else { 0 }));
            param_idx += 1;
        }

        let response = self.query(&sql, Some(params)).await?;
        
        match response.results {
            Some(results) => {
                let mut txs = Vec::new();
                for row in results {
                    txs.push(TxLifecycle {
                        extended_multi_id: row["extended_multi_id"]
                            .as_str()
                            .ok_or_else(|| anyhow!("Missing extended_multi_id"))?
                            .to_string(),
                        sender_address: row["sender_address"]
                            .as_str()
                            .ok_or_else(|| anyhow!("Missing sender_address"))?
                            .to_string(),
                        receiver_address: row["receiver_address"]
                            .as_str()
                            .ok_or_else(|| anyhow!("Missing receiver_address"))?
                            .to_string(),
                        tx_json: row["tx_json"]
                            .as_str()
                            .ok_or_else(|| anyhow!("Missing tx_json"))?
                            .to_string(),
                        receiver_confirmed: row["receiver_confirmed"]
                            .as_u64()
                            .unwrap_or(0) == 1,
                        reverted: row["reverted"].as_u64().unwrap_or(0) == 1,
                        completed: row["completed"].as_u64().unwrap_or(0) == 1,
                    });
                }
                Ok(txs)
            }
            None => Ok(vec![]),
        }
    }
}

// ============================================================================
// Metrics Counter Operations
// ============================================================================

/// Metrics counter record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsCounter {
    pub id: i32,
    pub sender_requests_total: i64,
    pub receiver_responses_total: i64,
    pub peers_added_total: i64,
    pub peers_removed_total: i64,
    pub new_clients_joined_total: i64,
}

impl D1Client {
    /// Get the metrics counter (singleton row with id=1)
    pub async fn get_metrics_counter(&self) -> Result<Option<MetricsCounter>> {
        let sql = "SELECT * FROM metrics_counter WHERE id = 1";
        let response = self.query(sql, None).await?;

        match response.results {
            Some(results) if !results.is_empty() => {
                let row = &results[0];
                Ok(Some(MetricsCounter {
                    id: row["id"].as_i64().unwrap_or(1) as i32,
                    sender_requests_total: row["sender_requests_total"].as_i64().unwrap_or(0),
                    receiver_responses_total: row["receiver_responses_total"].as_i64().unwrap_or(0),
                    peers_added_total: row["peers_added_total"].as_i64().unwrap_or(0),
                    peers_removed_total: row["peers_removed_total"].as_i64().unwrap_or(0),
                    new_clients_joined_total: row["new_clients_joined_total"].as_i64().unwrap_or(0),
                }))
            }
            _ => Ok(None),
        }
    }

    /// Increment sender_requests_total
    pub async fn increment_sender_requests(&self) -> Result<()> {
        let sql = "UPDATE metrics_counter SET sender_requests_total = sender_requests_total + 1 WHERE id = 1";
        self.execute(sql, None).await?;
        Ok(())
    }

    /// Increment receiver_responses_total
    pub async fn increment_receiver_responses(&self) -> Result<()> {
        let sql = "UPDATE metrics_counter SET receiver_responses_total = receiver_responses_total + 1 WHERE id = 1";
        self.execute(sql, None).await?;
        Ok(())
    }

    /// Increment peers_added_total
    pub async fn increment_peers_added(&self) -> Result<()> {
        let sql = "UPDATE metrics_counter SET peers_added_total = peers_added_total + 1 WHERE id = 1";
        self.execute(sql, None).await?;
        Ok(())
    }

    /// Increment peers_removed_total
    pub async fn increment_peers_removed(&self) -> Result<()> {
        let sql = "UPDATE metrics_counter SET peers_removed_total = peers_removed_total + 1 WHERE id = 1";
        self.execute(sql, None).await?;
        Ok(())
    }

    /// Increment new_clients_joined_total
    pub async fn increment_new_clients_joined(&self) -> Result<()> {
        let sql = "UPDATE metrics_counter SET new_clients_joined_total = new_clients_joined_total + 1 WHERE id = 1";
        self.execute(sql, None).await?;
        Ok(())
    }
}

