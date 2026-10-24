use std::{collections::HashSet, sync::Arc};

pub type Result<T> = std::result::Result<T, AgentError>;

#[derive(Debug, Clone, thiserror::Error)]
pub enum AgentError {
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("{0} is closed")]
    Closed(String),
    #[error("unknown branch: {0}")]
    UnknownBranch(String),
    #[error("storage: {0}")]
    Storage(String),
    #[error("initialization failed: {0}")]
    Build(String),
    #[error("operation cancelled: {0}")]
    Cancelled(String),
    /// A capability the operation needs is reloading; the operation did not run.
    #[error("{0} is reloading")]
    Reloading(String),
    #[error("{component}: {message}")]
    Fault { component: String, message: String },
    #[error("framework: {0}")]
    Framework(#[source] Arc<rutis::CordisError>),
    #[error("multiple failures: {0:?}")]
    Multiple(Vec<AgentError>),
}

impl AgentError {
    pub fn fault(component: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Fault {
            component: component.into(),
            message: message.into(),
        }
    }
    pub fn aggregate(errors: Vec<Self>) -> Result<()> {
        // A cached rutis failure can be observed through both construction and
        // parent cleanup. Merge those observations by Arc identity, never text.
        // collect only clones existing Arcs; it must not allocate new error
        // Arcs that could reuse an address of an already expanded wrapper.
        fn collect(
            error: AgentError,
            seen: &mut HashSet<*const rutis::CordisError>,
            out: &mut Vec<AgentError>,
        ) {
            match error {
                AgentError::Multiple(errors) if !errors.is_empty() => {
                    for error in errors {
                        collect(error, seen, out);
                    }
                }
                AgentError::Framework(error) => {
                    if !seen.insert(Arc::as_ptr(&error)) {
                        return;
                    }
                    match error.as_ref() {
                        rutis::CordisError::Aggregate { errors }
                        | rutis::CordisError::ListenersFailed { errors, .. }
                            if !errors.is_empty() =>
                        {
                            for error in errors {
                                collect(AgentError::Framework(error.clone()), seen, out);
                            }
                        }
                        rutis::CordisError::PluginFailed(source) => {
                            match source.downcast_ref::<AgentError>() {
                                Some(
                                    inner @ (AgentError::Multiple(_) | AgentError::Framework(_)),
                                ) => collect(inner.clone(), seen, out),
                                _ => out.push(AgentError::Framework(error)),
                            }
                        }
                        _ => out.push(AgentError::Framework(error)),
                    }
                }
                error => out.push(error),
            }
        }
        let mut unique = Vec::new();
        let mut seen = HashSet::new();
        for error in errors {
            collect(error, &mut seen, &mut unique);
        }
        let errors = unique;
        match errors.len() {
            0 => Ok(()),
            1 => Err(errors.into_iter().next().expect("one error")),
            _ => Err(Self::Multiple(errors)),
        }
    }
    pub fn with_cleanup(self, cleanup: Self) -> Self {
        Self::aggregate(vec![self, cleanup]).expect_err("construction and cleanup failed")
    }
}

/// Renders a caught panic payload into a diagnostic message.
pub fn panic_message(value: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = value.downcast_ref::<String>() {
        message.clone()
    } else if let Some(message) = value.downcast_ref::<&str>() {
        (*message).into()
    } else {
        "panic in managed operation".into()
    }
}
impl From<rutis::CordisError> for AgentError {
    fn from(error: rutis::CordisError) -> Self {
        Self::Framework(Arc::new(error))
    }
}
impl From<Arc<rutis::CordisError>> for AgentError {
    fn from(error: Arc<rutis::CordisError>) -> Self {
        Self::Framework(error)
    }
}
impl From<AgentError> for rutis::CordisError {
    fn from(error: AgentError) -> Self {
        Self::PluginFailed(Box::new(error))
    }
}
