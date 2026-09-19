//! Compatibility exports; shared usage mapping also supports Claude and Gemini.
pub use crate::formats::shared::simulated_cache::{
    apply_simulated_cache_usage_to_body as apply_simulated_cache_usage_to_openai_body,
    apply_simulated_cache_usage_to_body as apply_simulated_cache_usage_to_openai_responses_body,
    SimulatedCacheUsageStreamRewriter,
};
