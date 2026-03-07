use anyhow::{anyhow, Result};
use reqwest::Method;
use serde_json::{json, Value};

#[derive(Clone)]
pub struct CloudflareClient {
    token: String,
    account_id: String,
    zone_id: String,
    http: reqwest::Client,
}

#[derive(Debug, Clone)]
pub struct CreatedTunnel {
    pub id: String,
    pub token: String,
}

impl CloudflareClient {
    pub fn new(token: String, account_id: String, zone_id: String) -> Self {
        Self {
            token,
            account_id,
            zone_id,
            http: reqwest::Client::new(),
        }
    }

    pub fn is_configured(&self) -> bool {
        !self.token.trim().is_empty()
            && !self.account_id.trim().is_empty()
            && !self.zone_id.trim().is_empty()
    }

    async fn cf_json(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        if self.token.trim().is_empty() {
            return Err(anyhow!("CF_API_TOKEN is required"));
        }
        let url = format!("https://api.cloudflare.com/client/v4{}", path);
        let mut req = self
            .http
            .request(method, url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Content-Type", "application/json");
        if let Some(body) = body {
            req = req.json(&body);
        }
        let res = req.send().await?;
        let status = res.status();
        let payload: Value = res.json().await.unwrap_or_else(|_| json!({}));
        let success = payload
            .get("success")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !success {
            let msg = payload
                .get("errors")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("cloudflare error");
            return Err(anyhow!("{msg} (status={})", status.as_u16()));
        }
        Ok(payload.get("result").cloned().unwrap_or(Value::Null))
    }

    pub async fn create_tunnel(&self, name: &str) -> Result<CreatedTunnel> {
        if self.account_id.trim().is_empty() {
            return Err(anyhow!("CF_ACCOUNT_ID is required"));
        }
        let result = self
            .cf_json(
                Method::POST,
                &format!("/accounts/{}/cfd_tunnel", self.account_id),
                Some(json!({ "name": name })),
            )
            .await?;
        let id = result
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("cloudflare tunnel create missing id"))?
            .to_string();
        let token = result
            .get("token")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("cloudflare tunnel create missing token"))?
            .to_string();
        Ok(CreatedTunnel { id, token })
    }

    pub async fn find_tunnel_id_by_name(&self, name: &str) -> Result<Option<String>> {
        if self.account_id.trim().is_empty() {
            return Err(anyhow!("CF_ACCOUNT_ID is required"));
        }
        let name = name.trim();
        if name.is_empty() {
            return Ok(None);
        }

        // Cloudflare supports filtering by name; names used here are URL-safe already.
        let result = self
            .cf_json(
                Method::GET,
                &format!(
                    "/accounts/{}/cfd_tunnel?name={}&per_page=100",
                    self.account_id, name
                ),
                None,
            )
            .await?;
        let Some(items) = result.as_array() else {
            return Ok(None);
        };
        for item in items {
            let item_name = item.get("name").and_then(Value::as_str).unwrap_or("");
            if item_name == name {
                if let Some(id) = item.get("id").and_then(Value::as_str) {
                    return Ok(Some(id.to_string()));
                }
            }
        }
        Ok(None)
    }

    pub async fn delete_tunnel(&self, tunnel_id: &str) -> Result<()> {
        if self.account_id.trim().is_empty() {
            return Err(anyhow!("CF_ACCOUNT_ID is required"));
        }
        if tunnel_id.trim().is_empty() {
            return Ok(());
        }
        let _ = self
            .cf_json(
                Method::DELETE,
                &format!("/accounts/{}/cfd_tunnel/{}", self.account_id, tunnel_id),
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn get_tunnel_token(&self, tunnel_id: &str) -> Result<String> {
        if self.account_id.trim().is_empty() {
            return Err(anyhow!("CF_ACCOUNT_ID is required"));
        }
        if tunnel_id.trim().is_empty() {
            return Err(anyhow!("tunnel_id is required"));
        }
        let result = self
            .cf_json(
                Method::GET,
                &format!(
                    "/accounts/{}/cfd_tunnel/{}/token",
                    self.account_id, tunnel_id
                ),
                None,
            )
            .await?;
        let token = result
            .as_str()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| anyhow!("cloudflare tunnel token missing"))?;
        Ok(token.to_string())
    }

    pub async fn upsert_tunnel_dns_record(
        &self,
        hostname: &str,
        tunnel_id: &str,
        existing_record_id: Option<&str>,
    ) -> Result<String> {
        if self.zone_id.trim().is_empty() {
            return Err(anyhow!("CF_ZONE_ID is required"));
        }
        let hostname = hostname.trim();
        if hostname.is_empty() {
            return Err(anyhow!("hostname is required"));
        }
        let tunnel_id = tunnel_id.trim();
        if tunnel_id.is_empty() {
            return Err(anyhow!("tunnel_id is required"));
        }
        let payload = json!({
            "type": "CNAME",
            "name": hostname,
            "content": format!("{}.cfargotunnel.com", tunnel_id),
            "ttl": 1,
            "proxied": true,
        });

        // Helper: parse the record id from CF's result object.
        fn extract_id(result: &Value) -> Result<String> {
            let id = result
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .ok_or_else(|| anyhow!("cloudflare dns upsert missing id"))?;
            Ok(id.to_string())
        }

        // Attempt to update by id first if we have one.
        if let Some(id) = existing_record_id.map(str::trim).filter(|v| !v.is_empty()) {
            match self
                .cf_json(
                    Method::PUT,
                    &format!("/zones/{}/dns_records/{}", self.zone_id, id),
                    Some(payload.clone()),
                )
                .await
            {
                Ok(result) => return extract_id(&result),
                Err(err) => {
                    // Stored record id can go stale (manual deletes, migrations, zone changes).
                    // If the record doesn't exist anymore, fall back to creating or resolving
                    // the record by name.
                    let msg = err.to_string();
                    if !msg.contains("status=404") && !msg.contains("does not exist") {
                        return Err(err);
                    }
                }
            }
        }

        // Create new record. If it already exists (but we don't know the id), resolve by name and
        // update it.
        match self
            .cf_json(
                Method::POST,
                &format!("/zones/{}/dns_records", self.zone_id),
                Some(payload.clone()),
            )
            .await
        {
            Ok(result) => extract_id(&result),
            Err(err) => {
                let msg = err.to_string();
                // Common CF error: record already exists (then we need to find the id and update).
                if msg.contains("already exists")
                    || msg.contains("exists")
                    || msg.contains("status=409")
                {
                    if let Some(existing_id) = self.find_dns_record_id_by_name(hostname).await? {
                        let result = self
                            .cf_json(
                                Method::PUT,
                                &format!("/zones/{}/dns_records/{}", self.zone_id, existing_id),
                                Some(payload),
                            )
                            .await?;
                        return extract_id(&result);
                    }
                }
                Err(err)
            }
        }
    }

    async fn find_dns_record_id_by_name(&self, hostname: &str) -> Result<Option<String>> {
        if self.zone_id.trim().is_empty() {
            return Err(anyhow!("CF_ZONE_ID is required"));
        }
        let hostname = hostname.trim();
        if hostname.is_empty() {
            return Ok(None);
        }
        // Cloudflare supports filtering by name and type.
        let result = self
            .cf_json(
                Method::GET,
                &format!(
                    "/zones/{}/dns_records?type=CNAME&name={}&per_page=100",
                    self.zone_id, hostname
                ),
                None,
            )
            .await?;
        let Some(items) = result.as_array() else {
            return Ok(None);
        };
        for item in items {
            let name = item.get("name").and_then(Value::as_str).unwrap_or("");
            if name == hostname {
                if let Some(id) = item.get("id").and_then(Value::as_str) {
                    let id = id.trim();
                    if !id.is_empty() {
                        return Ok(Some(id.to_string()));
                    }
                }
            }
        }
        Ok(None)
    }
}
