//! Local Kotlin/JNI half of P5, not a remotely callable launch-authority API.
//! Main integrates `handle` in domains::dispatch and installs the Android bridge
//! result/consent sinks using the original authenticated peer's E4 context.
use serde_json::{json,Value};
use crate::mobile::{ApiError,Runtime};

#[cfg(target_os="android")]
pub(super) fn handle(method:&str,args:&Value)->Option<Result<Value,ApiError>> {
    use crate::apps::os::platform::bridge;
    let run=||->Result<Value,ApiError> {
        let error=|e:std::io::Error|ApiError::new("app_launch",e.to_string());
        let string=|key:&str|args.get(key).and_then(Value::as_str).ok_or_else(||ApiError::new("invalid",format!("Missing {key}")));
        match method {
            "apps.hostRegister"=>bridge::set_event_sink(emit).map_err(error)?,
            "apps.hostCatalog"=>{
                let catalog=serde_json::from_value(args.get("catalog").cloned().ok_or_else(||ApiError::new("invalid","Missing catalog"))?)
                    .map_err(|e|ApiError::new("invalid",e.to_string()))?;
                bridge::publish_catalog(string("challenge")?,catalog).map_err(error)?;
            },
            "apps.hostCatalogFailed"=>bridge::catalog_failed(string("challenge")?,string("reason")?).map_err(error)?,
            "apps.hostLaunchResult"=>{
                let state=serde_json::from_value(args.get("state").cloned().ok_or_else(||ApiError::new("invalid","Missing state"))?)
                    .map_err(|e|ApiError::new("invalid",e.to_string()))?;
                bridge::finish_launch(string("challenge")?,state,string("reason")?.into()).map_err(error)?;
            },
            "apps.hostConsent"=>bridge::confirm_local(string("request_id")?).map_err(error)?,
            "apps.hostDeny"=>bridge::deny_local(string("request_id")?).map_err(error)?,
            _=>return Err(ApiError::new("invalid","Unknown local app host method")),
        }
        Ok(json!({}))
    };
    if !matches!(method,"apps.hostRegister"|"apps.hostCatalog"|"apps.hostCatalogFailed"|"apps.hostLaunchResult"|"apps.hostConsent"|"apps.hostDeny") {return None;}
    Some(run())
}
#[cfg(not(target_os="android"))]
pub(super) fn handle(_method:&str,_args:&Value)->Option<Result<Value,ApiError>> {None}
fn emit(event:Value) {if let Some(rt)=Runtime::installed() {rt.emit(event);}}
