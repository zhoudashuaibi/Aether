use serde_json::Value;

pub(crate) use crate::ai_serving::api::apply_simulated_cache_usage_to_body;

use crate::ai_serving::api::SimulatedCacheUsageStreamRewriter;
use crate::ai_serving::{
    maybe_build_ai_surface_stream_rewriter, AiSurfaceFinalizeError, AiSurfaceStreamRewriter,
    ResponseHistoryRecord,
};
use crate::GatewayError;

pub(crate) struct LocalStreamRewriter<'a> {
    inner: Option<AiSurfaceStreamRewriter<'a>>,
    simulated_cache_usage: Option<SimulatedCacheUsageStreamRewriter>,
}

pub(crate) fn maybe_build_local_stream_rewriter<'a>(
    report_context: Option<&'a Value>,
) -> Option<LocalStreamRewriter<'a>> {
    let inner = maybe_build_ai_surface_stream_rewriter(report_context);
    let simulated_cache_usage =
        SimulatedCacheUsageStreamRewriter::from_report_context(report_context);
    (inner.is_some() || simulated_cache_usage.is_some()).then_some(LocalStreamRewriter {
        inner,
        simulated_cache_usage,
    })
}

impl LocalStreamRewriter<'_> {
    pub(crate) fn push_chunk(&mut self, chunk: &[u8]) -> Result<Vec<u8>, GatewayError> {
        let chunk = if let Some(inner) = self.inner.as_mut() {
            inner.push_chunk(chunk).map_err(map_surface_error)?
        } else {
            chunk.to_vec()
        };
        Ok(self
            .simulated_cache_usage
            .as_mut()
            .map(|rewriter| rewriter.push_chunk(&chunk))
            .unwrap_or(chunk))
    }

    pub(crate) fn finish(&mut self) -> Result<Vec<u8>, GatewayError> {
        let chunk = if let Some(inner) = self.inner.as_mut() {
            inner.finish().map_err(map_surface_error)?
        } else {
            Vec::new()
        };
        let mut output = self
            .simulated_cache_usage
            .as_mut()
            .map(|rewriter| rewriter.push_chunk(&chunk))
            .unwrap_or(chunk);
        if let Some(rewriter) = self.simulated_cache_usage.as_mut() {
            output.extend(rewriter.finish());
        }
        Ok(output)
    }

    pub(crate) fn take_response_history_record(&mut self) -> Option<ResponseHistoryRecord> {
        self.inner
            .as_mut()
            .and_then(AiSurfaceStreamRewriter::take_response_history_record)
    }
}

fn map_surface_error(error: AiSurfaceFinalizeError) -> GatewayError {
    error.into()
}

#[cfg(test)]
#[path = "../tests_stream.rs"]
mod tests;
