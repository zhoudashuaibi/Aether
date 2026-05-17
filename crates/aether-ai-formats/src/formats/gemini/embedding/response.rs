use serde_json::Value;

use crate::formats::openai::embedding::request::namespace_extensions;
use crate::protocol::canonical::{
    gemini_usage_to_canonical, CanonicalEmbedding, CanonicalEmbeddingResponse,
};

pub fn from(body_json: &Value) -> Option<CanonicalEmbeddingResponse> {
    let body = body_json.as_object()?;
    if body.contains_key("error") {
        return None;
    }

    let embeddings = if let Some(values) = body
        .get("embedding")
        .and_then(Value::as_object)
        .and_then(|embedding| embedding.get("values"))
        .and_then(Value::as_array)
    {
        vec![CanonicalEmbedding {
            index: 0,
            embedding: embedding_values(values)?,
            extensions: Default::default(),
        }]
    } else {
        let raw_embeddings = body.get("embeddings")?.as_array()?;
        raw_embeddings
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let item_object = item.as_object()?;
                let values = item_object.get("values")?.as_array()?;
                Some(CanonicalEmbedding {
                    index,
                    embedding: embedding_values(values)?,
                    extensions: namespace_extensions("gemini", item_object, &["values"]),
                })
            })
            .collect::<Option<Vec<_>>>()?
    };
    if embeddings.is_empty()
        || embeddings
            .iter()
            .any(|embedding| embedding.embedding.is_empty())
    {
        return None;
    }

    Some(CanonicalEmbeddingResponse {
        id: body
            .get("id")
            .or_else(|| body.get("responseId"))
            .and_then(Value::as_str)
            .unwrap_or("embd-gemini-unknown")
            .to_string(),
        model: body
            .get("model")
            .or_else(|| body.get("modelVersion"))
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        embeddings,
        usage: gemini_usage_to_canonical(body.get("usageMetadata")),
        extensions: namespace_extensions(
            "gemini",
            body,
            &[
                "id",
                "responseId",
                "model",
                "modelVersion",
                "embedding",
                "embeddings",
                "usageMetadata",
            ],
        ),
    })
}

fn embedding_values(values: &[Value]) -> Option<Vec<f64>> {
    values.iter().map(Value::as_f64).collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::from;

    #[test]
    fn parses_gemini_single_embedding_response() {
        let body = json!({
            "embedding": {"values": [0.1, 0.2, 0.3]},
            "usageMetadata": {
                "promptTokenCount": 4,
                "totalTokenCount": 4
            }
        });

        let parsed = from(&body).expect("response should parse");

        assert_eq!(parsed.model, "unknown");
        assert_eq!(parsed.embeddings[0].embedding, vec![0.1, 0.2, 0.3]);
        let usage = parsed.usage.expect("usage should parse");
        assert_eq!(usage.input_tokens, 4);
        assert_eq!(usage.total_tokens, 4);
    }
}
