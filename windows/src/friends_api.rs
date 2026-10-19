// SPDX-License-Identifier: MIT
//! 保存したキー（設定の窓で入れる。keystore.rs）を読む → GET /api/1/auth/user → friends の ID だけ返す。
//! キーはここで使うだけで、返さない・書かない・表示しない。
use std::sync::Arc;
use std::time::Duration;
use vwn::watcher::FetchResult;

pub fn fetch(ua: String) -> FetchResult {
    let mut r = FetchResult::default();
    let key = match crate::keystore::load() {
        Ok(Some(k)) => k,
        Ok(None) => {
            r.code = "nokey".into();
            return r;
        }
        Err(()) => {
            r.code = "unreadable".into();
            return r;
        }
    };
    let tls = match native_tls::TlsConnector::new() {
        Ok(t) => t,
        Err(_) => {
            r.code = "error".into();
            return r;
        }
    };
    let agent = ureq::AgentBuilder::new().tls_connector(Arc::new(tls)).timeout(Duration::from_secs(30)).build();
    let resp = agent
        .get("https://api.vrchat.cloud/api/1/auth/user")
        .set("User-Agent", &ua)
        .set("Accept", "application/json")
        .set("Cookie", &format!("auth={key}"))
        .call();
    drop(key);
    match resp {
        Ok(resp) => {
            let Ok(u) = resp.into_string().map_err(|_| ()).and_then(|b| serde_json::from_str::<serde_json::Value>(&b).map_err(|_| ())) else {
                r.code = "error".into();
                return r;
            };
            let Some(friends) = u["friends"].as_array() else {
                r.code = "nofriends".into(); // 2 段階認証の途中など
                return r;
            };
            r.ids = friends.iter().filter_map(|x| x.as_str()).filter(|s| s.starts_with("usr_")).map(String::from).collect();
            if let Some(id) = u["id"].as_str().filter(|s| s.starts_with("usr_")) {
                r.self_id = id.to_string();
            }
            r.ok = true;
            r.code = "200".into();
        }
        Err(ureq::Error::Status(code, _)) => r.code = code.to_string(),
        Err(_) => r.code = "net".into(),
    }
    r
}
