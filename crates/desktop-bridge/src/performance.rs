use crate::*;

impl DesktopBridge {
    pub async fn performance_get(&self) -> Result<PerformanceSnapshot> {
        // Observation is independent of model/download operations and never starts a service.
        if self.closing.load(Ordering::Acquire) {
            return Err(BridgeError::new("desktop_closing"));
        }
        let mut connection = self.connect().await?;
        let instance_id = connection.instance_id();
        let value = connection
            .json(Method::GET, "/runtime/performance", None)
            .await
            .map_err(|error| match error {
                runtime_cli::client::ClientError::Api { status: 404, .. } => {
                    BridgeError::new("performance_unsupported")
                }
                other => BridgeError::from(other),
            })?;
        decode(value, instance_id)
    }
}

fn decode(value: Value, instance_id: uuid::Uuid) -> Result<PerformanceSnapshot> {
    let snapshot: PerformanceSnapshot =
        serde_json::from_value(value).map_err(|_| BridgeError::new("response_invalid"))?;
    if snapshot.instance_id != instance_id || !snapshot.is_valid() {
        return Err(BridgeError::new("response_invalid"));
    }
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_is_bound_to_proved_instance_and_bounded() {
        let id = uuid::Uuid::new_v4();
        let value = json!({"instance_id": id, "capacity": 200, "records": []});
        assert!(decode(value.clone(), id).is_ok());
        assert!(decode(value.clone(), uuid::Uuid::new_v4()).is_err());
        let mut oversized = value.clone();
        oversized["capacity"] = json!(201);
        assert!(decode(oversized, id).is_err());
        let mut extra = value;
        extra["prompt"] = json!("unexpected");
        assert!(decode(extra, id).is_err());
    }
}
