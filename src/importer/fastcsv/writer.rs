use anyhow::{Result};
use tokio_postgres::Client;
use std::sync::Arc;
use tokio::sync::Mutex;
use futures_util::sink::SinkExt;
use bytes::Bytes;

#[derive(Clone)]
pub struct ShardWriter {
    client: Arc<Mutex<Client>>,
}

impl ShardWriter {
    pub fn new(_port: u16, client: Arc<Mutex<Client>>) -> Self {
        Self { client }
    }

    /// Executes high-performance COPY command for target tables
    pub async fn copy_records(
        &self,
        table_name: &str,
        columns: &[String],
        records: &[Vec<String>],
    ) -> Result<()> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await?;
        
        let sql = format!(
            "COPY public.{} ({}) FROM STDIN",
            table_name,
            columns.join(", ")
        );
        
        let sink = tx.copy_in(&sql).await?;
        tokio::pin!(sink);
        
        let mut buffer = String::with_capacity(records.len() * 128);
        for record in records {
            buffer.push_str(&record.join("\t"));
            buffer.push('\n');
        }
        
        sink.send(Bytes::from(buffer)).await?;
        
        sink.finish().await?;
        tx.commit().await?;
        Ok(())
    }
}
