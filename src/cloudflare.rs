use crate::public::credentials_file;
use crate::util::log;

const API: &str = "https://api.cloudflare.com/client/v4";

pub fn api_token() -> Option<String> {
    std::env::var("CLOUDFLARE_DNS_API_TOKEN")
        .ok()
        .filter(|s| !s.is_empty())
}

pub fn tunnel_id() -> Option<String> {
    let body = std::fs::read_to_string(credentials_file()).ok()?;
    let v: serde_json::Value = serde_json::from_str(&body).ok()?;
    v.get("TunnelID")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
}

fn auth(req: ureq::Request, token: &str) -> ureq::Request {
    req.set("Authorization", &format!("Bearer {token}"))
        .set("Accept", "application/json")
}

fn find_zone_id(token: &str, hostname: &str) -> Result<Option<String>, String> {
    let parts: Vec<&str> = hostname.split('.').collect();
    if parts.len() < 2 {
        return Ok(None);
    }
    for i in 0..=parts.len() - 2 {
        let zone = parts[i..].join(".");
        let url = format!("{API}/zones?name={zone}");
        let result = auth(ureq::get(&url), token).call();
        let body: serde_json::Value = match result {
            Ok(r) => r.into_json().map_err(|e| format!("cf parse zones: {e}"))?,
            Err(ureq::Error::Status(404, _)) => continue,
            Err(e) => return Err(format!("cf get zones: {e}")),
        };
        if let Some(id) = body
            .get("result")
            .and_then(|x| x.as_array())
            .and_then(|a| a.first())
            .and_then(|r| r.get("id"))
            .and_then(|x| x.as_str())
        {
            return Ok(Some(id.to_string()));
        }
    }
    Ok(None)
}

fn find_record_id(
    token: &str,
    zone_id: &str,
    name: &str,
    content: &str,
) -> Result<Option<String>, String> {
    let url = format!("{API}/zones/{zone_id}/dns_records?type=CNAME&name={name}&content={content}");
    let body: serde_json::Value = auth(ureq::get(&url), token)
        .call()
        .map_err(|e| format!("cf get records: {e}"))?
        .into_json()
        .map_err(|e| format!("cf parse records: {e}"))?;
    Ok(body
        .get("result")
        .and_then(|x| x.as_array())
        .and_then(|a| a.first())
        .and_then(|r| r.get("id"))
        .and_then(|x| x.as_str())
        .map(|s| s.to_string()))
}

fn delete_record(token: &str, zone_id: &str, record_id: &str) -> Result<(), String> {
    let url = format!("{API}/zones/{zone_id}/dns_records/{record_id}");
    auth(ureq::delete(&url), token)
        .call()
        .map_err(|e| format!("cf delete: {e}"))?;
    Ok(())
}

pub fn delete_dns_route(hostname: &str) -> Result<(), String> {
    let Some(token) = api_token() else {
        return Err("CLOUDFLARE_DNS_API_TOKEN not set".into());
    };
    let Some(tid) = tunnel_id() else {
        return Err("could not read tunnel id from credentials.json".into());
    };
    let content = format!("{tid}.cfargotunnel.com");
    let Some(zone_id) = find_zone_id(&token, hostname)? else {
        return Err(format!("no CF zone matches {hostname}"));
    };
    let Some(rid) = find_record_id(&token, &zone_id, hostname, &content)? else {
        return Ok(());
    };
    delete_record(&token, &zone_id, &rid)?;
    log(format!("cf: deleted CNAME {hostname} -> {content}"));
    Ok(())
}
