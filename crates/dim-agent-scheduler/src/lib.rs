//! Scheduled jobs for Dim, implemented natively. The design is
//! [docs/design/plugins/Scheduler.md](../../../docs/design/plugins/Scheduler.md).
//!
//! The process owner keeps jobs in the `schedule` settings document and run
//! history in `scheduler/runs.json`, and serves the `schedule` management
//! namespace.
mod document;
mod engine;
mod methods;
mod policy;
mod runs;
mod session;

use dim_agent_runtime::{AgentBuilder, Feature};
use rutis::host::DynPlugin;

pub use document::ScheduleDocument;
pub use methods::{ScheduleJobs, SchedulerPlugin};
pub use runs::{RUNS_FILE, SchedulerError};
pub use session::SchedulerSession;

/// Installs the scheduler: the process owner and the `schedule` tool. The
/// profile must include the App Server services it needs (settings, pluginFiles,
/// managementMethods and sessionPermissions).
pub fn install(builder: &mut AgentBuilder) {
    dim_agent_runtime::names::register::<ScheduleJobs>(builder.catalog_mut(), None);
    builder.plugin(|names| Ok(Box::new(SchedulerPlugin::new(names)?) as Box<dyn DynPlugin>));
    builder.require_feature(
        Feature::new("SchedulerTool").session("SchedulerSession", |names| {
            Ok(Box::new(SchedulerSession::new(names)?))
        }),
    );
}
