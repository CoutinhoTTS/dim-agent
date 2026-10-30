//! RFC-0023 recording of model wire exchanges, switched by the App Server's
//! `model-log` Settings source. Dim calls providers directly, so it also binds
//! the recorder to each request and closes the recording when the stream ends.
use aimux_core::recording::{
    JsonlRecorder, Recorder, RecordingContext, RecordingError, init_recording, recorder,
};
use std::{path::Path, sync::Arc};

/// One request's recorder and correlation id. The request carries the context;
/// this keeps what the stream's terminal needs.
pub(crate) struct Capture {
    pub(crate) recorder: Arc<dyn Recorder>,
    pub(crate) call_id: String,
}

impl Capture {
    pub(crate) fn input(
        &self,
        options: &aimux_core::options::CallOptions,
        provider: &str,
        model_id: &str,
    ) {
        self.recorder
            .record_input(&self.call_id, options, provider, model_id);
    }
}

/// Binds the request to the recorder that is on now, if any.
pub(crate) fn capture(
    call_id: String,
    options: &mut aimux_core::options::CallOptions,
) -> Option<Capture> {
    let recorder = recorder()?;
    options.call_id = Some(call_id.clone());
    options.recording_context = Some(RecordingContext::new(call_id.clone(), recorder.clone()));
    Some(Capture { recorder, call_id })
}

/// Writes every request's wire exchanges under `directory`, or stops when
/// `None`. Requests started afterwards carry the new state; one already in
/// flight finishes with the recorder it started with.
pub fn set_request_recording(directory: Option<&Path>) -> Result<(), RecordingError> {
    let recorder = directory
        .map(|directory| {
            JsonlRecorder::try_new(directory)
                .map(|recorder| Arc::new(recorder) as Arc<dyn Recorder>)
        })
        .transpose()?;
    init_recording(recorder);
    Ok(())
}
