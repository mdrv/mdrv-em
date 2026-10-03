//! Build-time embedding generator: data/catalog.json -> src/vectors.bin
//!
//! Embeds one line per emoji (name + CLDR keywords + LLM description if any)
//! with bge-small-en-v1.5 (int8 ONNX, embedded from assets/), writes a flat
//! little-endian f32 table, then prints sanity rankings for popular queries.
//!
//! Run: cargo run --bin gen-vectors
use fastembed::{
    InitOptionsUserDefined, Pooling, QuantizationMode, TextEmbedding, TokenizerFiles,
    UserDefinedEmbeddingModel,
};
use serde::Deserialize;
use std::fs;

#[derive(Deserialize)]
struct Entry {
    ch: String,
    name: String,
    #[serde(default)]
    desc: String,
    #[serde(default)]
    kws_en: Vec<String>,
}

const DIM: usize = 384;
/// bge v1.5 retrieval recipe: prefix queries, not passages.
const QUERY_PREFIX: &str = "Represent this sentence for searching relevant passages: ";

fn asset(name: &str) -> Vec<u8> {
    fs::read(format!("{}/assets/{}", env!("CARGO_MANIFEST_DIR"), name))
        .unwrap_or_else(|e| panic!("asset {name}: {e}"))
}

fn l2_normalize(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        for x in v {
            *x /= n;
        }
    }
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = env!("CARGO_MANIFEST_DIR");
    let catalog: Vec<Entry> =
        serde_json::from_reader(fs::File::open(format!("{root}/data/catalog.json"))?)?;
    let texts: Vec<String> = catalog
        .iter()
        .map(|e| {
            let mut t = e.name.clone();
            if !e.kws_en.is_empty() {
                t.push_str("; ");
                t.push_str(&e.kws_en.join(", "));
            }
            if !e.desc.is_empty() {
                t.push_str(". ");
                t.push_str(&e.desc);
            }
            t
        })
        .collect();

    let model = UserDefinedEmbeddingModel::new(
        asset("model_quantized.onnx"),
        TokenizerFiles {
            tokenizer_file: asset("tokenizer.json"),
            config_file: asset("config.json"),
            special_tokens_map_file: asset("special_tokens_map.json"),
            tokenizer_config_file: asset("tokenizer_config.json"),
        },
    )
    .with_pooling(Pooling::Cls)
    .with_quantization(QuantizationMode::Static);

    let mut engine =
        TextEmbedding::try_new_from_user_defined(model, InitOptionsUserDefined::new())?;
    let mut embeddings = engine.embed(&texts, Some(256))?;
    assert_eq!(embeddings.len(), catalog.len());
    assert_eq!(embeddings[0].len(), DIM, "unexpected embedding dim");

    for e in &mut embeddings {
        l2_normalize(e);
    }
    let mut buf = Vec::with_capacity(catalog.len() * DIM * 4);
    for e in &embeddings {
        for v in e {
            buf.extend_from_slice(&v.to_le_bytes());
        }
    }
    fs::write(format!("{root}/src/vectors.bin"), &buf)?;
    println!(
        "wrote src/vectors.bin: {} x {DIM} f32 ({} MB)",
        embeddings.len(),
        buf.len() / 1_048_576
    );

    let queries = ["youtube", "movie night", "love", "cat", "party"];
    let prefixed: Vec<String> = queries
        .iter()
        .map(|q| format!("{QUERY_PREFIX}{q}"))
        .collect();
    let q_emb = engine.embed(&prefixed, None)?;
    for (q, qv) in queries.iter().zip(&q_emb) {
        let mut qv = qv.clone();
        l2_normalize(&mut qv);
        let mut best: Vec<(f32, usize)> = embeddings
            .iter()
            .enumerate()
            .map(|(i, e)| (dot(&qv, e), i))
            .collect();
        best.sort_by(|a, b| b.0.total_cmp(&a.0));
        let tops: Vec<String> = best[..4]
            .iter()
            .map(|(s, i)| format!("{} {} {:.3}", catalog[*i].ch, catalog[*i].name, s))
            .collect();
        println!("{q:>12} -> {}", tops.join(" | "));
    }
    Ok(())
}
