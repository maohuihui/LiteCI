use crate::LogEvent;

pub const MAX_LOG_BYTES: usize = 8 * 1024 * 1024;
const MAX_LOG_CHUNK_BYTES: usize = 64 * 1024;

pub async fn persist_stage_logs(
    pool: sqlx::SqlitePool,
    stage_run_id: String,
    mut receiver: tokio::sync::mpsc::Receiver<LogEvent>,
) -> Result<bool, sqlx::Error> {
    let mut sequence = 0_i64;
    let mut total_bytes = 0_usize;
    let mut truncated = false;

    while let Some(event) = receiver.recv().await {
        let stream = match event.stream {
            crate::LogStream::Stdout => "stdout",
            crate::LogStream::Stderr => "stderr",
        };
        for chunk in event.data.chunks(MAX_LOG_CHUNK_BYTES) {
            let remaining = MAX_LOG_BYTES.saturating_sub(total_bytes);
            if remaining == 0 {
                truncated = true;
                continue;
            }
            let length = chunk.len().min(remaining);
            if length < chunk.len() {
                truncated = true;
            }
            sqlx::query("INSERT INTO pipeline_stage_logs (id, stage_run_id, sequence, stream, data) VALUES (?1, ?2, ?3, ?4, ?5)")
                .bind(uuid::Uuid::new_v4().to_string())
                .bind(&stage_run_id)
                .bind(sequence)
                .bind(stream)
                .bind(&chunk[..length])
                .execute(&pool)
                .await?;
            total_bytes += length;
            sequence += 1;
        }
    }
    Ok(truncated)
}
