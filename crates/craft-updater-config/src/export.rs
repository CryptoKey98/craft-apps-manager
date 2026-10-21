use crate::ResolvedConfig;
use anyhow::Result;
#[derive(Clone, Copy)]
pub enum View {
    Build,
    Packaging,
    Publisher,
    Worker,
}
impl ResolvedConfig {
    pub fn view(&self, view: View) -> Result<serde_json::Value> {
        let mut value = serde_json::to_value(self)?;
        value["runtime_config_fingerprint"] = self.runtime_fingerprint().into();
        value["view"] = match view {
            View::Build => "build",
            View::Packaging => "packaging",
            View::Publisher => "publisher",
            View::Worker => "worker",
        }
        .into();
        if matches!(view, View::Worker) {
            let w = &self.config.worker;
            value["worker_config"] = serde_json::json!({"name":w.name,"main":"src/index.ts","compatibility_date":"2026-10-16","workers_dev":true,"assets":{"directory":"./public","binding":"ASSETS","run_worker_first":["/v2/check"],"not_found_handling":"404-page","html_handling":"none"},"ratelimits":[{"name":"UPDATES_RATE_LIMITER","namespace_id":w.rate_limit_namespace,"simple":{"limit":w.rate_limit_requests,"period":w.rate_limit_period_seconds}}],"vars":{"DYNAMIC_ENABLED":w.dynamic_enabled.to_string(),"UPDATE_PUBLIC_KEY":self.public_key_hex}});
        }
        Ok(value)
    }
}
