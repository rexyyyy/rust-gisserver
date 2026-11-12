//! 引擎后台巡检：周期探测启用引擎的存活与服务状态，变化写 `sys_engine_event`。
//!
//! * 引擎存活：探测 `/admin/components`（退 `/health`），翻转记
//!   `engine_online` / `engine_offline`；
//! * 服务清单 diff：与上次快照比较 (kind,name) 集合与 (state,serving)：
//!   新增 → `service_added`，消失 → `service_removed`，
//!   serving 翻转 → `service_up` / `service_down`；
//! * 首轮只建快照不记事件（避免重启刷屏）。
//!
//! 周期 30s；巡检失败静默（尽力而为），下次周期自动重试。
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::handlers::engine;
use crate::state::AppState;

/// 巡检周期（秒）。
const INTERVAL_SECS: u64 = 30;

/// 启动后台巡检任务。
pub fn spawn(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(INTERVAL_SECS));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // 引擎存活快照：engine_id -> up
        let mut eng_snap: HashMap<String, bool> = HashMap::new();
        // 服务快照："engine_id/kind/name" -> (state, serving)
        let mut svc_snap: HashMap<String, (String, bool)> = HashMap::new();
        let mut first = true;
        loop {
            ticker.tick().await;
            run_once(&state, &mut eng_snap, &mut svc_snap, first).await;
            first = false;
        }
    });
}

/// 单轮巡检。
async fn run_once(
    state: &AppState,
    eng_snap: &mut HashMap<String, bool>,
    svc_snap: &mut HashMap<String, (String, bool)>,
    first: bool,
) {
    for e in engine::enabled_rows(state).await {
        // 1) 引擎存活探测
        let health = engine::probe(state, &e).await;
        let up = health
            .get("up")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        match eng_snap.get(&e.engine_id) {
            Some(prev) if *prev != up => {
                let (ev, summary) = if up {
                    ("engine_online", format!("引擎上线：{}", e.engine_name))
                } else {
                    ("engine_offline", format!("引擎离线：{}", e.engine_name))
                };
                engine::log_event(state, &e.engine_id, ev, &summary, &health.to_string()).await;
            }
            None => {}
            _ => {}
        }
        eng_snap.insert(e.engine_id.clone(), up);

        if !up {
            // 引擎离线时服务快照保留，等上线后按快照对比（不误报 service_down）
            continue;
        }

        // 2) 服务清单 diff
        let base = e.base_url.trim_end_matches('/');
        let token = e.admin_token.as_deref().filter(|t| !t.is_empty());
        let svc = match engine::fetch_services_of(state, base, token).await {
            Ok(v) => v,
            Err(err) => {
                tracing::debug!("巡检拉取服务清单失败（{}）：{err}", e.engine_id);
                continue;
            }
        };
        let list = svc
            .get("services")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut seen: Vec<String> = Vec::with_capacity(list.len());
        for s in &list {
            let (kind, name) = match (
                s.get("kind").and_then(|v| v.as_str()),
                s.get("name").and_then(|v| v.as_str()),
            ) {
                (Some(k), Some(n)) => (k, n),
                _ => continue,
            };
            let state_s = s.get("state").and_then(|v| v.as_str()).unwrap_or("");
            let serving = s.get("serving").and_then(|v| v.as_bool()).unwrap_or(false);
            let key = format!("{}/{}/{}", e.engine_id, kind, name);
            seen.push(key.clone());
            let entry = (state_s.to_string(), serving);
            match svc_snap.get(&key) {
                None => {
                    if !first {
                        engine::log_event(
                            state,
                            &e.engine_id,
                            "service_added",
                            &format!("服务登记：{kind}/{name}（引擎 {}）", e.engine_name),
                            &format!("state={state_s}, serving={serving}"),
                        )
                        .await;
                    }
                }
                Some((prev_state, prev_serving)) => {
                    if prev_state != state_s {
                        engine::log_event(
                            state,
                            &e.engine_id,
                            "service_state",
                            &format!("服务状态变更：{kind}/{name}（{} → {state_s}）", prev_state),
                            "",
                        )
                        .await;
                    }
                    if *prev_serving != serving {
                        let (ev, summary) = if serving {
                            ("service_up", format!("服务恢复在服：{kind}/{name}"))
                        } else {
                            ("service_down", format!("服务离线：{kind}/{name}"))
                        };
                        engine::log_event(state, &e.engine_id, ev, &summary, "").await;
                    }
                }
            }
            svc_snap.insert(key, entry);
        }
        // 消失的服务（本轮快照里已不在清单）
        let gone: Vec<String> = svc_snap
            .keys()
            .filter(|k| k.starts_with(&format!("{}/", e.engine_id)) && !seen.contains(k))
            .cloned()
            .collect();
        for key in gone {
            svc_snap.remove(&key);
            if !first {
                let short = key
                    .splitn(3, '/')
                    .nth(1)
                    .map(|rest| rest.to_string())
                    .unwrap_or(key.clone());
                engine::log_event(
                    state,
                    &e.engine_id,
                    "service_removed",
                    &format!("服务移除：{short}"),
                    "",
                )
                .await;
            }
        }
    }
}
