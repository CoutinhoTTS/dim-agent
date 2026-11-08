/// The hard bounds of one Code Mode program. A request may ask for a lower
/// timeout or call budget; it can never raise these.
#[derive(Debug, Clone)]
pub struct CodeModeConfig {
    pub max_code_bytes: usize,
    pub max_runtime_ms: u64,
    pub max_calls: u64,
    pub max_parallel_calls: usize,
    pub max_memory_bytes: usize,
    pub max_output_bytes: usize,
    pub max_single_result_bytes: usize,
}

impl Default for CodeModeConfig {
    fn default() -> Self {
        Self {
            max_code_bytes: 64 * 1024,
            max_runtime_ms: 30_000,
            max_calls: 64,
            max_parallel_calls: 4,
            max_memory_bytes: 256 * 1024 * 1024,
            max_output_bytes: 1024 * 1024,
            max_single_result_bytes: 256 * 1024,
        }
    }
}
